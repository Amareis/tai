use crate::agent::{Agent, AgentResponse};
use crate::backend::Backend;
use crate::response::edit_command;
use crate::session::SessionDir;
use crate::state::State;
use crate::types::{BlockMode, ParsedBlock};
use std::collections::HashMap;
use std::fmt::Write as _;
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
    pub max_ticks: Option<u64>,
    pub tui: bool,
    pub no_delegate: bool,
}

impl Server {
    #[must_use]
    pub fn new(session: SessionDir, back: Box<dyn Backend>, agent: Box<dyn Agent>) -> Self {
        Self {
            session,
            back,
            agent,
            debug: false,
            max_ticks: None,
            tui: false,
            no_delegate: false,
        }
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        info!("run: starting server");

        let (mut state, mut response) = {
            let segments = self.session.read_index().await;
            let outputs = self.session.read_all_outputs(&segments).await;
            let system = self.session.read_system_prompt().await;
            let tick = self.session.read_tick(0).await;
            let response = self
                .session
                .read_tick_response(tick)
                .await
                .unwrap_or_default();

            let mut state = State::build(system, segments, outputs, tick);
            state.task.clone_from(&response.task);
            (state, response)
        };

        info!(
            "run: {} initial segments, entering tick loop",
            state.segments.len()
        );

        loop {
            state.system = self.session.read_system_prompt().await;

            let prev_state = state.clone();
            let result = {
                let tick_fut = self.tick_tack(&mut state, Some(response));

                tokio::select! {
                    res = tick_fut => res,
                    _ = tokio::signal::ctrl_c() => {
                        info!("run: ctrl+c received, exiting");
                        Err(CoreError::Interrupted)
                    }
                }
            };

            // Persist output changes and current state
            self.sync_outputs(&prev_state.outputs, &state.outputs).await;
            self.commit_state(&state).await;

            match result {
                Err(e) => {
                    // Persist state since apply_response already applied side effects
                    if !matches!(e, CoreError::Interrupted) {
                        warn!("tick error: {}", e);
                    }
                    if !state.task.is_empty() {
                        println!("\n{}", state.task);
                    }
                    if self.tui {
                        self.session.print_banner();
                    }
                    return Err(e);
                }
                Ok(None) => {
                    // Session completed
                    if !state.task.is_empty() {
                        println!("\n{}", state.task);
                    }
                    break;
                }
                Ok(Some(new_response)) => {
                    response = new_response;
                    self.session
                        .write_tick_response(state.tick_n, &response)
                        .await;
                }
            }

            if let Some(max) = self.max_ticks
                && state.tick_n >= max
            {
                break;
            }

            if self.debug {
                if self.tui {
                    self.session.print_banner();
                }
                break;
            }
        }

        Ok(())
    }

    pub async fn tick_tack(
        &mut self,
        state: &mut State,
        resp: Option<AgentResponse>,
    ) -> Result<Option<AgentResponse>, CoreError> {
        let completed = if let Some(resp) = resp {
            self.apply_response(state, resp).await
        } else {
            state.is_completed
        };

        let tick_n = state.tick_n;
        if completed {
            info!("tick #{tick_n}: state completed");
            return Ok(None);
        }

        info!(
            "tick #{tick_n}: state updated (now {} segments), calling agent",
            state.segments.len()
        );

        let response = self.agent.step(state).await?;
        info!(
            "tick #{tick_n}: agent responded ({} segments, task {} bytes)",
            response.segments.len(),
            response.task.len()
        );

        info!("tick #{tick_n}: done");
        Ok(Some(response))
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_response(&self, state: &mut State, response: AgentResponse) -> bool {
        if state.is_complete() {
            return true;
        }

        state.tick_n += 1;
        state.task.clone_from(&response.task);

        let State {
            outputs,
            segments,
            tick_n,
            ..
        } = state;

        info!("tick #{tick_n}: start");

        let mut close_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut ask_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut exec_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut watch_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut file_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut write_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut edit_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut delegate_blocks: Vec<&ParsedBlock> = Vec::new();

        for block in &response.segments {
            match block.mode {
                BlockMode::Close => close_blocks.push(block),
                BlockMode::Ask => ask_blocks.push(block),
                BlockMode::Exec => exec_blocks.push(block),
                BlockMode::Watch => watch_blocks.push(block),
                BlockMode::File => file_blocks.push(block),
                BlockMode::Write => write_blocks.push(block),
                BlockMode::Edit(_) => edit_blocks.push(block),
                BlockMode::Delegate => delegate_blocks.push(block),
                BlockMode::Task => {}
            }
        }

        for block in &close_blocks {
            outputs.remove(&block.window);
            segments.retain(|s| s.window != block.window);
        }

        for block in &ask_blocks {
            let answer = ask_user(&block.content).await;
            info!("answer: '{}'", answer);
            let output = crate::backend::CmdOutput {
                exit_code: 0,
                stdout: answer,
            };
            outputs.insert(block.window.clone(), output);
            upsert_segment(segments, (*block).clone());
        }

        for block in &exec_blocks {
            debug!("execute: '{}' exec", block.window);
            let output = self.back.run(&block.window, &block.content).await;
            outputs.insert(block.window.clone(), output);
            upsert_segment(segments, (*block).clone());
        }

        for block in &write_blocks {
            debug!("execute: '{}' write", block.window);
            let cmd = format!(
                "cat > {} << 'TAIWRITE'\n{}\nTAIWRITE",
                block.window, block.content
            );
            let _output = self.back.run(&block.window, &cmd).await;

            let file_view = self.back.file(&block.window).await;
            outputs.insert(block.window.clone(), file_view);

            move_to_end(segments, &block.window);
            upsert_segment(segments, (*block).clone());
        }

        let mut edit_groups: HashMap<String, Vec<&ParsedBlock>> = HashMap::new();
        for block in &edit_blocks {
            edit_groups
                .entry(block.window.clone())
                .or_default()
                .push(block);
        }

        for (path, blocks) in &edit_groups {
            let prev_output = outputs.get(path).cloned();

            let mut all_commands = Vec::new();
            let mut edit_err: Option<edit_command::EditError> = None;

            for block in blocks {
                if let BlockMode::Edit(ref cmds) = block.mode {
                    if cmds.is_empty() {
                        edit_err = Some(edit_command::EditError::Parse {
                            line: 0,
                            message: "edit commands not parsed".to_string(),
                        });
                        break;
                    }
                    if let Some(ref out) = prev_output
                        && let Err(e) =
                            edit_command::validate_texts_against_output(cmds, &out.stdout)
                    {
                        edit_err = Some(e);
                        break;
                    }
                    all_commands.extend(cmds.iter().cloned());
                }
            }

            if let Some(e) = edit_err {
                warn!("edit error for {path}: {e}");
                let output = crate::backend::CmdOutput {
                    exit_code: 1,
                    stdout: format!("edit error: {e}"),
                };
                outputs.insert(path.clone(), output);
                continue;
            }

            if all_commands.is_empty() {
                continue;
            }

            edit_command::sort_bottom_up(&mut all_commands);
            let script = edit_command::build_ex_script(path, &all_commands);
            debug!("execute: '{}' edit script: {}", path, script);

            let _output = self.back.run(path, &script).await;

            let file_view = self.back.file(path).await;
            outputs.insert(path.clone(), file_view);

            move_to_end(segments, path);
            if let Some(first) = blocks.first() {
                upsert_segment(segments, (*first).clone());
            }
        }
        for block in &delegate_blocks {
            if outputs.contains_key(&block.window) {
                debug!("delegate: '{}' cached", block.window);
                continue;
            }
            if self.no_delegate {
                outputs.insert(
                    block.window.clone(),
                    crate::backend::CmdOutput {
                        exit_code: 1,
                        stdout: "delegate disabled".to_string(),
                    },
                );
                upsert_segment(segments, (*block).clone());
                continue;
            }
            debug!("delegate: '{}' starting", block.window);
            let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("tai"));
            let child_path = self
                .session
                .session_path()
                .join("delegates")
                .join(&block.window);
            let mut cmd = format!(
                "{} --session {}",
                shell_escape(&exe.to_string_lossy()),
                shell_escape(&child_path.to_string_lossy()),
            );
            if let Some(max) = self.max_ticks {
                let _ = write!(cmd, " --max-ticks {max}");
            }
            let _ = write!(cmd, " --no-delegate");
            let _ = write!(cmd, " <<'TAIEOF'\n{}\nTAIEOF", block.content);
            let output = self.back.run(&block.window, &cmd).await;
            outputs.insert(block.window.clone(), output);
            upsert_segment(segments, (*block).clone());
        }

        for block in file_blocks {
            upsert_segment(segments, block.clone());
        }

        for block in watch_blocks {
            upsert_segment(segments, (*block).clone());
        }

        for block in segments {
            if block.mode == BlockMode::Watch || block.mode == BlockMode::File {
                debug!("execute: '{}' watch", block.window);
                let output = if block.mode == BlockMode::File {
                    self.back.file(&block.window).await
                } else {
                    self.back.run(&block.window, &block.content).await
                };
                outputs.insert(block.window.clone(), output);
            }
        }

        state.is_completed = response.complete;
        state.is_completed
    }

    async fn sync_outputs(
        &self,
        old: &HashMap<String, crate::backend::CmdOutput>,
        new: &HashMap<String, crate::backend::CmdOutput>,
    ) {
        for key in old.keys() {
            if !new.contains_key(key) {
                self.session.remove_out(key).await;
            }
        }
        for (key, val) in new {
            if old.get(key) != Some(val) {
                self.session.write_out(key, val).await;
            }
        }
    }

    async fn commit_state(&self, state: &State) {
        self.session.write_index(&state.segments).await;
        self.session.write_tick(state.tick_n).await;
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
    println!("\n? {question}");
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

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
