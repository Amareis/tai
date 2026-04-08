use std::fmt::Write;
use std::time::Duration;

use rustyline_async::ReadlineError;
use thiserror::Error;
use tokio::net::UnixListener;
use tokio::select;
use tokio::time::sleep;
use tracing::info;

use crate::backend::watch::{WatchEvent, Watcher};
use crate::backend::{BackendCmd, CmdResponse, LaunchCmd, TerminalBackend, WindowId};
use crate::models::Agent;
use crate::routing::parser;
use crate::types::{BlockMode, ParsedSegment, Session};
use connection::Connection;

mod client;
mod connection;
pub mod utils;

pub use client::Client;

const TRIGGER_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("connection closed")]
    ConnectionClosed,

    #[error("readline error: {0}")]
    Readline(#[from] ReadlineError),

    #[error("agent error: {0}")]
    Agent(#[from] crate::models::AgentError),

    #[error("backend error: {0}")]
    Backend(#[from] crate::backend::BackendError),

    #[error("prompt error: {0}")]
    Prompt(#[from] crate::prompt::PromptError),
}

enum Trigger {
    WatcherEvents(Vec<WatchEvent>),
    UserInput(String),
    Timeout,
}

pub struct Server {
    client: Connection,
    back: Box<dyn TerminalBackend>,
    session: Session,
    agent: Box<dyn Agent>,
    watcher: Watcher,
}

impl Server {
    pub async fn accept(
        unix_listener: &UnixListener,
        back: Box<dyn TerminalBackend>,
        session: Session,
        agent: Box<dyn Agent>,
    ) -> Result<Self, CoreError> {
        Connection::accept(unix_listener).await.map(|client| Self {
            client,
            back,
            session,
            agent,
            watcher: Watcher::new(),
        })
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        self.client
            .write_line("════════════════════════════════════════════════════════════")
            .await?;
        self.client
            .write_line("STATE: Active 0 | Frozen 0 | Tokens: 0")
            .await?;
        self.client
            .write_line("════════════════════════════════════════════════════════════")
            .await?;
        self.client.write_line("").await?;
        self.client
            .write_line("TAI Server ready. Type commands in this window.")
            .await?;
        self.client.write_line("").await?;

        info!("model viewport initialized");

        self.tick().await?;

        loop {
            match self.wait_trigger().await? {
                Trigger::WatcherEvents(events) => {
                    for event in &events {
                        info!("watcher event: {event:?}");
                    }
                    self.tick().await?;
                }
                Trigger::UserInput(line) => {
                    self.handle_input(&line).await?;
                }
                Trigger::Timeout => {
                    // nothing happened, loop back to wait
                }
            }
        }
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        let events = self.watcher.poll(self.back.as_ref(), &mut self.session).await?;

        for event in &events {
            info!("watcher event: {event:?}");
        }

        let prompt = crate::prompt::build(&self.session, self.back.as_ref(), None).await?;

        let response = self.agent.step(&prompt).await?;

        let results = self.execute_blocks(&response.segments).await;

        let output = format_results(&response.segments, &results);
        self.client.write_line(&output).await?;
        Ok(())
    }

    async fn wait_trigger(&mut self) -> Result<Trigger, CoreError> {
        let events = self.watcher.poll(self.back.as_ref(), &mut self.session).await?;
        if !events.is_empty() {
            return Ok(Trigger::WatcherEvents(events));
        }

        select! {
            result = self.client.read_line() => {
                match result {
                    Ok(Some(line)) => Ok(Trigger::UserInput(line)),
                    Ok(None) => Err(CoreError::ConnectionClosed),
                    Err(e) => Err(e),
                }
            }
            () = sleep(TRIGGER_TIMEOUT) => Ok(Trigger::Timeout),
        }
    }

    async fn handle_input(&mut self, line: &str) -> Result<(), CoreError> {
        info!("received from model channel: {line}");

        match parser::parse(line) {
            Ok(cmd) => match self.back.execute(cmd).await {
                Ok(response) => match response {
                    CmdResponse::WindowCreated(id) => {
                        self.client.write_line(&format!("Window created: {id}")).await?;
                    }
                    CmdResponse::Text(text) => {
                        self.client.write_line(&text).await?;
                    }
                    CmdResponse::Windows(windows) => {
                        self.client
                            .write_line(&format!("{} windows:", windows.len()))
                            .await?;
                        for w in windows {
                            self.client
                                .write_line(&format!(
                                    "  {} | {} | pid {} | prompt: {}",
                                    w.id, w.title, w.pid, w.is_at_prompt
                                ))
                                .await?;
                        }
                    }
                    CmdResponse::Ok => {
                        self.client.write_line("OK").await?;
                    }
                    CmdResponse::Error(e) => {
                        self.client.write_line(&format!("Error: {e}")).await?;
                    }
                },
                Err(e) => {
                    self.client.write_line(&format!("Backend error: {e}")).await?;
                }
            },
            Err(e) => {
                self.client.write_line(&format!("{e}")).await?;
            }
        }

        Ok(())
    }

    async fn execute_blocks(&mut self, segments: &[ParsedSegment]) -> Vec<BlockResult> {
        let mut results = Vec::new();

        for segment in segments {
            if let ParsedSegment::Block {
                window,
                mode,
                content,
            } = segment
            {
                let result = self.execute_block(window, mode, content).await;
                results.push(result);
            }
        }

        results
    }

    async fn execute_block(
        &mut self,
        window: &str,
        mode: &BlockMode,
        content: &str,
    ) -> BlockResult {
        match mode {
            BlockMode::Text => {
                let cmd = BackendCmd::Send(crate::backend::SendTextCmd {
                    window: WindowId(window.to_string()),
                    text: vec![content.to_string()],
                });
                self.execute_backend_cmd(cmd, window).await
            }
            BlockMode::Keys => {
                let keys: Vec<String> = content.split_whitespace().map(String::from).collect();
                let cmd = BackendCmd::Keys(crate::backend::SendKeysCmd {
                    window: WindowId(window.to_string()),
                    keys,
                });
                self.execute_backend_cmd(cmd, window).await
            }
            BlockMode::Cmd => match parser::parse(content) {
                Ok(BackendCmd::Launch(launch_cmd)) => {
                    self.execute_launch(launch_cmd).await
                }
                Ok(cmd) => self.execute_backend_cmd(cmd, "tai").await,
                Err(e) => BlockResult::Error("tai".to_string(), e.to_string()),
            },
        }
    }

    async fn execute_launch(&mut self, cmd: LaunchCmd) -> BlockResult {
        let title = cmd.title.as_deref().unwrap_or("tai").to_string();
        match self.back.execute(BackendCmd::Launch(cmd)).await {
            Ok(CmdResponse::WindowCreated(id)) => {
                self.watcher.track(id.clone(), title);
                BlockResult::Created(id)
            }
            Ok(CmdResponse::Error(e)) => BlockResult::Error("tai".to_string(), e),
            Ok(_) => BlockResult::Error("tai".to_string(), "unexpected response".to_string()),
            Err(e) => BlockResult::Error("tai".to_string(), e.to_string()),
        }
    }

    async fn execute_backend_cmd(&mut self, cmd: BackendCmd, target: &str) -> BlockResult {
        match self.back.execute(cmd).await {
            Ok(CmdResponse::WindowCreated(id)) => BlockResult::Created(id),
            Ok(CmdResponse::Text(text)) => BlockResult::Text(target.to_string(), text),
            Ok(CmdResponse::Windows(windows)) => {
                BlockResult::List(target.to_string(), windows.len())
            }
            Ok(CmdResponse::Ok) => BlockResult::Ok(target.to_string()),
            Ok(CmdResponse::Error(e)) => BlockResult::Error(target.to_string(), e),
            Err(e) => BlockResult::Error(target.to_string(), e.to_string()),
        }
    }
}

enum BlockResult {
    Created(WindowId),
    Text(String, String),
    List(String, usize),
    Ok(String),
    Error(String, String),
}

fn format_results(segments: &[ParsedSegment], results: &[BlockResult]) -> String {
    let mut output = String::new();

    for segment in segments {
        if let ParsedSegment::Prose(text) = segment {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(text);
        }
    }

    for result in results {
        if !output.is_empty() {
            output.push('\n');
        }
        match result {
            BlockResult::Created(id) => {
                let _ = write!(output, "[created] {id}");
            }
            BlockResult::Text(target, text) => {
                let _ = write!(output, "[{target}] {text}");
            }
            BlockResult::List(target, count) => {
                let _ = write!(output, "[{target}] {count} windows");
            }
            BlockResult::Ok(target) => {
                let _ = write!(output, "[{target}] OK");
            }
            BlockResult::Error(target, e) => {
                let _ = write!(output, "[{target}] Error: {e}");
            }
        }
    }

    output
}

