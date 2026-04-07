use std::fmt::Write;
use std::time::Duration;

use rustyline_async::ReadlineError;
use thiserror::Error;
use tokio::net::UnixListener;
use tokio::select;
use tokio::time::sleep;
use tracing::info;

use crate::backend::{BackendCmd, CmdResponse, TerminalBackend, WindowId};
use crate::models::Agent;
use crate::response::parse_response;
use crate::routing::parser;
use crate::types::{BlockMode, ParsedSegment, Session};
use connection::Connection;

mod client;
mod connection;
pub mod utils;

pub use client::Client;

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

    #[error("parse error: {0}")]
    Parse(#[from] parser::ParseError),

    #[error("prompt error: {0}")]
    Prompt(#[from] crate::prompt::PromptError),
}

pub struct Server {
    client: Connection,
    back: Box<dyn TerminalBackend>,
    session: Session,
    agent: Box<dyn Agent>,
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

        self.tui_loop().await
    }

    async fn tick(&mut self) -> Result<(), CoreError> {
        let prompt = crate::prompt::build(&self.session, self.back.as_ref(), None).await?;

        let response = self.agent.step(&prompt).await?;

        let segments = parse_response(&response);
        let results = self.execute_blocks(&segments).await;

        let output = format_results(&segments, &results);
        self.client.write_line(&output).await?;
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
                Ok(cmd) => self.execute_backend_cmd(cmd, "tai").await,
                Err(e) => BlockResult::Error("tai".to_string(), e.to_string()),
            },
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

    async fn tui_loop(&mut self) -> Result<(), CoreError> {
        let mut exit = false;

        while !exit {
            select! {
                should_exit = client_loop(&mut self.client, self.back.as_mut()) => {
                    match should_exit {
                        Ok(e) => {exit = e}
                        Err(e) => {
                            tracing::error!("error reading from model channel: {}", e);
                            return Err(e)
                        }
                    }
                }
                () = sleep(Duration::from_millis(60)) => {}
            }
        }
        Ok(())
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

async fn client_loop(
    client: &mut Connection,
    back: &mut dyn TerminalBackend,
) -> Result<bool, CoreError> {
    if let Some(line) = client.read_line().await? {
        info!("received from model channel: {}", line);

        match parser::parse(&line) {
            Ok(cmd) => match back.execute(cmd).await {
                Ok(response) => match response {
                    CmdResponse::WindowCreated(id) => {
                        client.write_line(&format!("Window created: {id}")).await?;
                    }
                    CmdResponse::Text(text) => {
                        client.write_line(&text).await?;
                    }
                    CmdResponse::Windows(windows) => {
                        client
                            .write_line(&format!("{} windows:", windows.len()))
                            .await?;
                        for w in windows {
                            client
                                .write_line(&format!(
                                    "  {} | {} | pid {} | prompt: {}",
                                    w.id, w.title, w.pid, w.is_at_prompt
                                ))
                                .await?;
                        }
                    }
                    CmdResponse::Ok => {
                        client.write_line("OK").await?;
                    }
                    CmdResponse::Error(e) => {
                        client.write_line(&format!("Error: {e}")).await?;
                    }
                },
                Err(e) => {
                    client.write_line(&format!("Backend error: {e}")).await?;
                }
            },
            Err(e) => {
                client.write_line(&format!("{e}")).await?;
            }
        }

        Ok(false)
    } else {
        info!("model channel closed");
        Err(CoreError::ConnectionClosed)
    }
}
