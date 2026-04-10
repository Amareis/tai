use std::fmt::Write;
use std::time::Duration;

use crate::agent::Agent;
use crate::backend::watch::Watcher;
use crate::backend::{BackendCmd, CmdResponse, LaunchCmd, TerminalBackend, WindowId};
use crate::routing::{ParseError, parser};
use crate::types::{ParsedSegment, TickTrigger};
use connection::Connection;
use rustyline_async::ReadlineError;
use thiserror::Error;
use tokio::select;
use tracing::info;

mod client;
pub mod connection;
pub mod utils;

use crate::prompt::Prompt;
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
    pub debug: bool,
}

impl Server {
    #[must_use]
    pub fn new(client: Connection, back: Box<dyn TerminalBackend>, agent: Box<dyn Agent>) -> Self {
        Self {
            client,
            back,
            agent,
            watcher: Watcher::new(),
            debug: false,
        }
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        self.client
            .write_line("TAI Server ready. Type commands in this window.")
            .await?;

        info!("model viewport initialized");
        self.run_cmd("title 1 tai").await;
        self.run_cmd("help").await;
        self.run_cmd("launch -t TASK -- watch -t cat TASK.md").await;

        if !self.debug {
            self.tick().await?;
        }

        loop {
            let _events = self.wait_trigger(Some(TRIGGER_TIMEOUT)).await?;
            if !self.debug {
                self.tick().await?;
            }
        }
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        let prompt = Prompt::build(self.back.as_ref(), None).await?;

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
        let exited = self.watcher.poll_exited(self.back.as_ref()).await?;
        if !exited.is_empty() {
            info!("watcher exited: {exited:?}");
            return Ok(TickTrigger::WindowsExited(exited));
        }

        select! {
            result = self.client.read_line() => {
                info!("client line: {result:?}");
                match result {
                    Ok(Some(line)) => {
                        if line == "ai" {
                            self.tick().await?;
                        } else {
                            self.run_cmd(&line).await;
                        }
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

    async fn run_cmd(&mut self, content: &str) {
        let seg = vec![ParsedSegment::Block {
            window: "tai".to_string(),
            content: content.to_string(),
        }];
        let res = self.execute_blocks(&seg).await;
        let _ = self.client.write_line(&format_results(&seg, &res)).await;
    }

    async fn execute_blocks(&mut self, segments: &[ParsedSegment]) -> Vec<BlockResult> {
        let mut results = Vec::new();

        for segment in segments {
            if let ParsedSegment::Block { window, content } = segment {
                let result = self.execute_block(window, content).await;
                results.push(result);
            }
        }

        results
    }

    async fn execute_block(&mut self, window: &str, content: &str) -> BlockResult {
        if window == "tai" {
            self.execute_tai_command(content).await
        } else {
            self.execute_window_block(window, content).await
        }
    }

    async fn execute_tai_command(&mut self, content: &str) -> BlockResult {
        let _ = self.client.write_line(content).await;
        let mut last_result = BlockResult::Ok("tai".to_string());

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match parser::parse(trimmed) {
                Ok(cmd) => {
                    last_result = self.execute_backend_cmd(cmd, "tai").await;
                }
                Err(e) => {
                    last_result = if let ParseError::Help(help) = e {
                        BlockResult::Text("tai".to_string(), help)
                    } else {
                        BlockResult::Error("tai".to_string(), e.to_string())
                    };
                }
            }
        }

        last_result
    }

    async fn get_id_for_title(&mut self, title: &str) -> Option<WindowId> {
        match self.back.execute(BackendCmd::List).await {
            Ok(CmdResponse::Windows(windows)) => windows
                .iter()
                .find(|w| w.title == title)
                .map(|w| w.id.clone()),
            _ => None,
        }
    }

    async fn execute_window_block(&mut self, title: &str, content: &str) -> BlockResult {
        let id = if let Some(id) = self.get_id_for_title(title).await {
            id
        } else {
            let launch_cmd = LaunchCmd {
                title: Some(title.to_string()),
                command: vec!["zsh".to_string()],
            };
            let id = match self.back.cmd_launch(&launch_cmd).await {
                Ok(id) => id,
                Err(e) => {
                    return BlockResult::Error(title.to_string(), format!("launch failed: {e}"));
                }
            };
            let _ = self
                .client
                .write_line(&format!("[{title}] launched new window with id {id}"))
                .await;
            id
        };

        let _ = self
            .client
            .write_line(&format!("```{title}\n{content}```"))
            .await;

        let text_to_send = format!("{content}\n");
        let cmd = BackendCmd::Send(crate::backend::SendTextCmd {
            window: id,
            text: vec![text_to_send],
        });
        self.execute_backend_cmd(cmd, title).await
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
                windows
                    .iter()
                    .map(|w| {
                        format!(
                            "{} | {} | pid {} | prompt: {}",
                            w.id, w.title, w.pid, w.is_at_prompt
                        )
                    })
                    .collect(),
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
