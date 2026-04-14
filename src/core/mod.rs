use crate::agent::{Agent, AgentResponse};
use crate::backend::Backend;
use crate::session::SessionDir;
use crate::state::State;
use crate::types::{BlockMode, ParsedBlock};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("agent error: {0}")]
    Agent(#[from] crate::agent::AgentError),
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

        let mut segments = self.session.read_index();
        if segments.is_empty() {
            info!("run: no initial segments, exiting");
            self.print_banner();
            return Ok(());
        }

        info!(
            "run: {} initial segments, entering tick loop",
            segments.len()
        );

        loop {
            let result = {
                let tick_fut = self.tick(&mut segments);
                tokio::pin!(tick_fut);

                tokio::select! {
                    res = &mut tick_fut => Some(res),
                    _ = tokio::signal::ctrl_c() => {
                        info!("run: ctrl+c received, exiting");
                        None
                    }
                }
            };

            match result {
                None => {
                    self.session.write_index(&segments);
                    break;
                }
                Some(res) => res?,
            }

            if segments.is_empty() {
                info!("run: no segments left, exiting");
                break;
            }
        }

        self.print_banner();
        Ok(())
    }

    fn print_banner(&self) {
        let to_resume = format!("  tai server {}  ", self.session.session_path().display());
        let width = std::cmp::max(50, to_resume.chars().count());
        let border = "═".repeat(width);
        println!();
        println!("╔{:═^width$}╗", " TO RESUME SESSION ");
        println!("║{:^width$}║", " ");
        println!("║{to_resume:^width$}║");
        println!("║{:^width$}║", " ");
        println!("╚{border}╝");
    }

    pub async fn tick(&mut self, segments: &mut Vec<ParsedBlock>) -> Result<(), CoreError> {
        let tick_n = self.session.read_tick() + 1;
        self.session.write_tick(tick_n);
        info!("tick #{tick_n}: start");

        self.debug_wait("before execute").await;

        let outputs = self.execute_segments(segments).await;

        let system = self.session.read_system_prompt();
        let state = State::build(system, segments.clone(), outputs, tick_n);
        info!("tick #{tick_n}: state built, calling agent");

        self.debug_wait("before agent call").await;

        let response = self.agent.step(&state).await?;
        info!(
            "tick #{tick_n}: agent responded ({} segments)",
            response.segments.len()
        );

        self.session.write_response(&response);

        self.session.append_to_mind(&response.reasoning);

        *segments = merge_segments(segments, &response);
        self.session.write_index(segments);

        info!("tick #{tick_n}: done");
        Ok(())
    }

    async fn execute_segments(
        &mut self,
        segments: &[ParsedBlock],
    ) -> HashMap<String, crate::backend::CmdOutput> {
        let mut outputs: HashMap<String, crate::backend::CmdOutput> = HashMap::new();

        let mut ask_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut exec_blocks: Vec<&ParsedBlock> = Vec::new();
        let mut view_blocks: Vec<&ParsedBlock> = Vec::new();

        for block in segments {
            match block.mode {
                BlockMode::Ask => ask_blocks.push(block),
                BlockMode::Close => {
                    self.session.remove_out(&block.window);
                }
                BlockMode::Exec => exec_blocks.push(block),
                BlockMode::View => view_blocks.push(block),
            }
        }

        for block in &ask_blocks {
            if self.session.has_out(&block.window) {
                debug!("execute: '{}' ask cached", block.window);
                if let Some(output) = self.session.read_out(&block.window) {
                    outputs.insert(block.window.clone(), output);
                }
            } else {
                let answer = self.ask_user(&block.content).await;
                let output = crate::backend::CmdOutput {
                    exit_code: 0,
                    stdout: answer,
                };
                self.session.write_out(&block.window, &output);
                outputs.insert(block.window.clone(), output);
            }
        }

        for block in &exec_blocks {
            if self.session.has_out(&block.window) {
                debug!("execute: '{}' cached", block.window);
                if let Some(output) = self.session.read_out(&block.window) {
                    outputs.insert(block.window.clone(), output);
                }
            } else {
                debug!("execute: '{}' exec", block.window);
                let output = self.back.run(&block.window, &block.content).await;
                self.session.write_out(&block.window, &output);
                outputs.insert(block.window.clone(), output);
            }
        }

        for block in &view_blocks {
            debug!("execute: '{}' view", block.window);
            let output = self.back.run(&block.window, &block.content).await;
            self.session.write_out(&block.window, &output);
            outputs.insert(block.window.clone(), output);
        }

        outputs
    }

    async fn ask_user(&self, question: &str) -> String {
        use std::io::Write;
        println!("\n❓ {question}");
        print!("> ");
        std::io::stdout().flush().ok();
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut answer = String::new();
        let _ = reader.read_line(&mut answer).await;
        answer.trim_end().to_string()
    }

    async fn debug_wait(&self, point: &str) {
        if !self.debug {
            return;
        }
        info!("debug: press Enter to continue ({point})...");
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        let _ = lines.next_line().await;
    }
}

#[must_use]
fn merge_segments(old: &[ParsedBlock], response: &AgentResponse) -> Vec<ParsedBlock> {
    let mut closed: HashSet<String> = HashSet::new();
    let mut updated: HashSet<String> = HashSet::new();

    for block in &response.segments {
        match block.mode {
            BlockMode::Close => {
                closed.insert(block.window.clone());
            }
            _ => {
                updated.insert(block.window.clone());
            }
        }
    }

    let mut result: Vec<ParsedBlock> = Vec::new();

    for block in old {
        if !closed.contains(&block.window) && !updated.contains(&block.window) {
            result.push(ParsedBlock {
                prose: None,
                ..block.clone()
            });
        }
    }

    for block in &response.segments {
        if block.mode != BlockMode::Close {
            result.push(block.clone());
        }
    }

    result
}
