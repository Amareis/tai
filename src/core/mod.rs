use crate::agent::{Agent};
use crate::backend::Backend;
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

            // todo move to tick somehow
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

        if state.segments.is_empty() {
            info!("tick #{tick_n}: state updated, no segments left, exiting");
            return Ok(state);
        }

        info!("tick #{tick_n}: state updated, calling agent");

        self.debug_wait("before agent call").await;

        let response = self.agent.step(&state).await?;
        info!(
            "tick #{tick_n}: agent responded ({} segments)",
            response.segments.len()
        );
        state.response = response;
        state.tick_n += 1;

        info!("tick #{tick_n}: done");
        Ok(state)
    }

    async fn update_state(&self, state: &mut State) {
        let State {
            response,
            outputs,
            segments,
            ..
        } = state;
        let mut ask_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut exec_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut view_blocks: Vec<&ParsedBlock> = Vec::new();

        for block in &response.segments {
            match block.mode {
                BlockMode::Ask => ask_blocks.push(block),
                BlockMode::Close => {
                    self.session.remove_out(&block.window);
                    outputs.remove(&block.window);
                    segments.retain(|s| s.window != block.window);
                }
                BlockMode::Exec => exec_blocks.push(block),
                BlockMode::View => view_blocks.push(block),
            }
        }

        for block in ask_blocks {
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
                segments.push(block.clone());
            }
        }

        for block in exec_blocks {
            if let Some(output) = self.session.read_out(&block.window) {
                debug!("execute: '{}' cached", block.window);
                outputs.insert(block.window.clone(), output);
            } else {
                debug!("execute: '{}' exec", block.window);
                let output = self.back.run(&block.window, &block.content).await;
                self.session.write_out(&block.window, &output);
                outputs.insert(block.window.clone(), output);
                segments.push(block.clone());
            }
        }

        for block in view_blocks {
            segments.push(block.clone());
        }

        for block in segments {
            if block.mode == BlockMode::View {
                debug!("execute: '{}' view", block.window);
                let output = self.back.run(&block.window, &block.content).await;
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
