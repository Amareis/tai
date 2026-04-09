use std::fmt::Write;
use std::time::Duration;

use crate::agent::Agent;
use crate::backend::watch::Watcher;
use crate::backend::{BackendCmd, CmdResponse, TerminalBackend, WindowId};
use crate::routing::parser;
use crate::types::{BlockMode, ParsedSegment, TickTrigger};
use connection::Connection;
use rustyline_async::ReadlineError;
use thiserror::Error;
use tokio::select;
use tracing::info;

mod client;
pub mod connection;
pub mod utils;

pub use client::Client;
use utils::sleep_some_or_forever;

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
    Agent(#[from] crate::agent::AgentError),

    #[error("backend error: {0}")]
    Backend(#[from] crate::backend::BackendError),

    #[error("prompt error: {0}")]
    Prompt(#[from] crate::prompt::PromptError),
}

pub struct Server {
    client: Connection,
    back: Box<dyn TerminalBackend>,
    pub agent: Box<dyn Agent>,
    watcher: Watcher,
}

impl Server {
    #[must_use]
    pub fn new(
        client: Connection,
        back: Box<dyn TerminalBackend>,
        agent: Box<dyn Agent>,
    ) -> Self {
        Self {
            client,
            back,
            agent,
            watcher: Watcher::new(),
        }
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
            let _events = self.wait_trigger(Some(TRIGGER_TIMEOUT)).await?;
            self.tick().await?;
        }
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        let events = self
            .watcher
            .poll_exited(self.back.as_ref())
            .await?;

        for event in &events {
            info!("watcher event: {event:?}");
        }

        let prompt = crate::prompt::build(self.back.as_ref(), None).await?;

        let response = self.agent.step(&prompt).await?;

        let results = self.execute_blocks(&response.segments).await;

        let output = format_results(&response.segments, &results);

        if !output.is_empty() {
            self.client.write_line(&output).await?;
        }
        Ok(())
    }

    pub async fn wait_trigger(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<TickTrigger, CoreError> {
        let exited = self
            .watcher
            .poll_exited(self.back.as_ref())
            .await?;
        if !exited.is_empty() {
            info!("watcher exited: {exited:?}");
            return Ok(TickTrigger::WindowsExited(exited));
        }

        select! {
            result = self.client.read_line() => {
                info!("client line: {result:?}");
                match result {
                    Ok(Some(line)) => {
                        let seg = vec![ParsedSegment::Block{window: "tai".to_string(), mode: BlockMode::Cmd, content:line}];
                        let res = self.execute_blocks(&seg).await;
                        let _ = self.client.write_line(&format_results(&seg, &res)).await;
                        Ok(TickTrigger::UserMessage)
                    },
                    Ok(None) => Err(CoreError::ConnectionClosed),
                    Err(e) => Err(e),
                }
            }
            () = sleep_some_or_forever(timeout) => {
                Ok(TickTrigger::IdleTimeout)
            },
        }
    }
/*
    async fn handle_input(&mut self, line: &str) -> Result<(), CoreError> {
        info!("received from model channel: {line}");

        match parser::parse(line) {
            Ok(cmd) => match self.back.execute(cmd).await {
                Ok(response) => match response {
                    CmdResponse::WindowCreated(id) => {
                        self.client
                            .write_line(&format!("Window created: {id}"))
                            .await?;
                    }
                    CmdResponse::Text(text) => {
                        if !text.is_empty() {
                            self.client.write_line(&text).await?;
                        }
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
                    self.client
                        .write_line(&format!("Backend error: {e}"))
                        .await?;
                }
            },
            Err(e) => {
                self.client.write_line(&e.to_string()).await?;
            }
        }

        Ok(())
    }
*/
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
                let _ = self.client.write_line(&format!("send {window} {content}")).await;
                let cmd = BackendCmd::Send(crate::backend::SendTextCmd {
                    window: WindowId(window.to_string()),
                    text: vec![content.to_string()],
                });
                self.execute_backend_cmd(cmd, window).await
            }
            BlockMode::Keys => {
                let _ = self.client.write_line(&format!("keys {window} {content}")).await;
                let keys: Vec<String> = content.split_whitespace().map(String::from).collect();
                let cmd = BackendCmd::Keys(crate::backend::SendKeysCmd {
                    window: WindowId(window.to_string()),
                    keys,
                });
                self.execute_backend_cmd(cmd, window).await
            }
            BlockMode::Cmd => {
                let _ = self.client.write_line(content).await;
                match parser::parse(content) {
                    Ok(cmd) => self.execute_backend_cmd(cmd, "tai").await,
                    Err(e) => BlockResult::Error("tai".to_string(), e.to_string()),
                }
            }
        }
    }

    async fn execute_backend_cmd(&mut self, cmd: BackendCmd, target: &str) -> BlockResult {
        match self.back.execute(cmd).await {
            Ok(CmdResponse::WindowCreated(id)) => {
                self.watcher.track(id.clone(), true);
                BlockResult::Created(id)
            }
            Ok(CmdResponse::Text(text)) => BlockResult::Text(target.to_string(), text),
            Ok(CmdResponse::Windows(windows)) => BlockResult::List(
                target.to_string(),
                windows.iter().map(|w| {
                    format!(
                        "{} | {} | pid {} | prompt: {}",
                        w.id, w.title, w.pid, w.is_at_prompt
                    )
                }).collect(),
            ),
            Ok(CmdResponse::Ok) => BlockResult::Ok(target.to_string()),
            Ok(CmdResponse::Error(e)) => BlockResult::Error(target.to_string(), e),
            Err(e) => BlockResult::Error(target.to_string(), e.to_string()),
        }
    }
}

enum BlockResult {
    Created(WindowId),
    Text(String, String),
    List(String, Vec<String>),
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
                let _ = write!(output, "[created], id = {id}");
            }
            BlockResult::Text(target, text) => {
                let _ = write!(output, "[{target}] {text}");
            }
            BlockResult::List(target, list) => {
                let _ = write!(output, "[{target}]\n{}", list.join("\n"));
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
