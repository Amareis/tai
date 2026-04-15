use crate::agent::Agent;
use crate::backend::Backend;
use crate::response::edit_command;
use crate::session::SessionDir;
use crate::state::State;
use crate::types::{BlockMode, ParsedBlock};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("agent error: {0}")]
    Agent(#[from] crate::agent::AgentError),

    #[error("edit error: {0}")]
    Edit(#[from] edit_command::EditError),

    #[error("Ctrl-C received")]
    Interrupted,
}

pub struct Server {
    session: SessionDir,
    back: Box<dyn Backend>,
    pub agent: Box<dyn Agent>,
    pub debug: bool,
}

impl Server {
    #[must_use]
    pub fn new(session: SessionDir, _back: Box<dyn Agent>, agent: Box<dyn Agent>) -> Self {
        let cwd = session.workspace().to_path_buf();
        Self {
            session,
            back: Box::new(crate::backend::local::LocalBackend::new(cwd)),
            agent,
            debug: false,
        }
    }

    #[must_use]
    pub fn with_backend(
        session: SessionDir,
        back: Box<dyn Backend>,
        agent: Box<dyn Agent>,
    ) -> Self {
        Self {
            session,
            back,
            agent,
            debug: false,
        }
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.session.read_index().len()
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        info!("run: starting server");

        let mut state = {
            let segments = self.session.read_index();
            let outputs = self.session.read_all_outputs(&segments);
            let system = self.session.read_system_prompt().await;
            let tick = self.session.read_tick(1).await;
            let response = self.session.read_response().unwrap_or_default();

            State::build(system, segments, outputs, tick, response)
        };

        info!(
            "run: {} initial segments, entering tick loop",
            state.segments.len()
        );

        loop {
            state.system = self.session.read_system_prompt().await;

            let result = {
                let tick_fut = self.tick(state);
                tokio::pin!(tick_fut);

                tokio::select! {
                    res = &mut tick_fut => res,
                    _ = tokio::signal::ctrl_c() => {
                        info!("run: ctrl+c received, exiting");
                        Err(CoreError::Interrupted)
                    }
                }
            };

            match result {
                Err(e) => {
                    if !matches!(e, CoreError::Interrupted) {
                        warn!("tick error: {}", e);
                    }
                    self.session.print_banner();
                    break;
                }
                Ok(s) => state = s,
            }

            self.session.write_response(&state.response);

            self.session.append_to_mind(&state.response.reasoning);

            self.session.write_index(&state.segments);

            self.session.write_tick(state.tick_n);

            if state.segments.is_empty() {
                break;
            }
        }

        Ok(())
    }

    pub async fn tick(&mut self, mut state: State) -> Result<State, CoreError> {
        let tick_n = state.tick_n;
        info!("tick #{tick_n}: start");

        self.debug_wait("before update_state").await;

        self.update_state(&mut state).await;

        info!(
            "tick #{tick_n}: state updated ({} segments), calling agent",
            state.segments.len()
        );

        self.debug_wait("before agent call").await;

        let response = self.agent.step(&state).await?;
        info!(
            "tick #{tick_n}: agent responded ({} segments)",
            response.segments.len()
        );

        let mut pending_segments: Vec<ParsedBlock> = Vec::new();
        for block in &response.segments {
            match block.mode {
                BlockMode::Close => {
                    state.segments.retain(|s| s.window != block.window);
                }
                _ => {
                    if let Some(pos) =
                        state.segments.iter().position(|s| s.window == block.window)
                    {
                        if let Some(seg) = state.segments.get_mut(pos) {
                            *seg = block.clone();
                        }
                    } else {
                        pending_segments.push(block.clone());
                    }
                }
            }
        }
        state.segments.extend(pending_segments);

        state.response = response;
        state.tick_n += 1;

        info!("tick #{tick_n}: done");
        Ok(state)
    }

    #[allow(clippy::too_many_lines)]
    async fn update_state(&self, state: &mut State) {
        let State {
            response,
            outputs,
            segments,
            ..
        } = state;
        let mut close_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut ask_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut exec_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut watch_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut file_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut write_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut edit_blocks: Vec<&ParsedBlock> = Vec::new();

        for block in &response.segments {
            match block.mode {
                BlockMode::Close => close_blocks.push(block),
                BlockMode::Ask => ask_blocks.push(block),
                BlockMode::Exec => exec_blocks.push(block),
                BlockMode::Watch => watch_blocks.push(block),
                BlockMode::File => file_blocks.push(block),
                BlockMode::Write => write_blocks.push(block),
                BlockMode::Edit => edit_blocks.push(block),
            }
        }

        for block in &close_blocks {
            self.session.remove_out(&block.window);
            outputs.remove(&block.window);
            segments.retain(|s| s.window != block.window);
        }

        for block in &ask_blocks {
            if let Some(output) = self.session.read_out(&block.window) {
                debug!("execute: '{}' ask cached", block.window);
                outputs.insert(block.window.clone(), output);
            } else {
                let answer = ask_user(&block.content).await;
                info!("answer: '{}'", answer);
                let output = crate::backend::CmdOutput {
                    exit_code: 0,
                    stdout: answer,
                };
                self.session.write_out(&block.window, &output);
                outputs.insert(block.window.clone(), output);
                upsert_segment(segments, (*block).clone());
            }
        }

        for block in &exec_blocks {
            if let Some(output) = self.session.read_out(&block.window) {
                debug!("execute: '{}' cached", block.window);
                outputs.insert(block.window.clone(), output);
            } else {
                debug!("execute: '{}' exec", block.window);
                let output = self.back.run(&block.window, &block.content).await;
                self.session.write_out(&block.window, &output);
                outputs.insert(block.window.clone(), output);
                upsert_segment(segments, (*block).clone());
            }
        }

        for block in &write_blocks {
            self.session.remove_out(&block.window);
            outputs.remove(&block.window);
            debug!("execute: '{}' write", block.window);
            let cmd = format!(
                "cat > {} << 'TAIWRITE'\n{}\nTAIWRITE",
                block.window, block.content
            );
            let output = self.back.run(&block.window, &cmd).await;
            self.session.write_out(&block.window, &output);

            let file_view = self.back.run(&block.window, &format!("cat -n {}", block.window)).await;
            outputs.insert(block.window.clone(), file_view);

            move_to_end(segments, &block.window);
            upsert_segment(segments, (*block).clone());
        }

        let mut edit_groups: std::collections::HashMap<String, Vec<&ParsedBlock>> =
            std::collections::HashMap::new();
        for block in &edit_blocks {
            edit_groups
                .entry(block.window.clone())
                .or_default()
                .push(block);
        }

        for (path, blocks) in &edit_groups {
            self.session.remove_out(path);
            outputs.remove(path);

            let mut all_commands = Vec::new();
            for block in blocks {
                match edit_command::parse_edit_commands(&block.content) {
                    Ok(cmds) => all_commands.extend(cmds),
                    Err(e) => {
                        warn!("edit parse error for {path}: {e}");
                        let output = crate::backend::CmdOutput {
                            exit_code: 1,
                            stdout: format!("edit error: {e}"),
                        };
                        self.session.write_out(path, &output);
                        outputs.insert(path.clone(), output);
                    }
                }
            }

            if all_commands.is_empty() {
                continue;
            }

            edit_command::sort_bottom_up(&mut all_commands);
            let script = edit_command::build_ex_script(path, &all_commands);
            debug!("execute: '{}' edit script: {}", path, script);

            let output = self.back.run(path, &script).await;
            self.session.write_out(path, &output);

            let file_view = self
                .back
                .run(path, &format!("cat -n {path}"))
                .await;
            outputs.insert(path.clone(), file_view);

            move_to_end(segments, path);
            if let Some(first) = blocks.first() {
                upsert_segment(segments, (*first).clone());
            }
        }

        for block in &file_blocks {
            let watch_block = ParsedBlock {
                window: block.window.clone(),
                mode: BlockMode::Watch,
                content: format!("cat -n {}", block.window),
                prose: block.prose.clone(),
            };
            upsert_segment(segments, watch_block);
        }

        for block in watch_blocks {
            upsert_segment(segments, (*block).clone());
        }

        for block in segments {
            if block.mode == BlockMode::Watch || block.mode == BlockMode::File {
                debug!("execute: '{}' watch", block.window);
                let cmd = if block.mode == BlockMode::File {
                    format!("cat -n {}", block.window)
                } else {
                    block.content.clone()
                };
                let output = self.back.run(&block.window, &cmd).await;
                self.session.write_out(&block.window, &output);
                outputs.insert(block.window.clone(), output);
            }
        }
    }

    async fn debug_wait(&self, point: &str) {
        if !self.debug {
            return;
        }
        println!("debug ({point}): press Enter to continue...");
        read_line().await;
    }
}

fn upsert_segment(segments: &mut Vec<ParsedBlock>, block: ParsedBlock) {
    if let Some(pos) = segments.iter().position(|s| s.window == block.window) {
        if let Some(seg) = segments.get_mut(pos) {
            *seg = block;
        }
    } else {
        segments.push(block);
    }
}

fn move_to_end(segments: &mut Vec<ParsedBlock>, window: &str) {
    if let Some(idx) = segments.iter().position(|s| s.window == window) {
        let seg = segments.remove(idx);
        segments.push(seg);
    }
}

async fn ask_user(question: &str) -> String {
    use std::io::Write;
    println!("\n❓ {question}");
    print!("> ");
    std::io::stdout().flush().ok();
    read_line().await.trim_end().to_string()
}

async fn read_line() -> String {
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    lines
        .next_line()
        .await
        .unwrap_or_else(|e| Some(e.to_string()))
        .unwrap_or_else(|| "NONE".to_string())
}
