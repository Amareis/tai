use std::fmt::Write;

use crate::backend::{BackendCmd, CmdResponse, TerminalBackend, WindowId};
use crate::models::Agent;
use crate::response::parse_response;
use crate::routing::parser;
use crate::types::{BlockMode, ParsedSegment, Session};

pub struct Kernel<'a, A: Agent> {
    session: Session,
    backend: &'a dyn TerminalBackend,
    agent: A,
}

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("agent error: {0}")]
    Agent(#[from] crate::models::AgentError),
    #[error("backend error: {0}")]
    Backend(#[from] crate::backend::BackendError),
    #[error("parse error: {0}")]
    Parse(#[from] parser::ParseError),
    #[error("prompt error: {0}")]
    Prompt(#[from] crate::prompt::PromptError),
}

impl<'a, A: Agent + Send + Sync> Kernel<'a, A> {
    pub fn new(session: Session, backend: &'a dyn TerminalBackend, agent: A) -> Self {
        Self {
            session,
            backend,
            agent,
        }
    }

    pub async fn tick(&mut self) -> Result<String, KernelError> {
        let prompt = crate::prompt::build(&self.session, self.backend, None).await?;

        let response = self.agent.step(&prompt).await?;

        let segments = parse_response(&response);
        let results = self.execute_blocks(&segments).await;

        let output = format_results(&segments, &results);
        Ok(output)
    }

    async fn execute_blocks(&self, segments: &[ParsedSegment]) -> Vec<BlockResult> {
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
        &self,
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

    async fn execute_backend_cmd(&self, cmd: BackendCmd, target: &str) -> BlockResult {
        match self.backend.execute(cmd).await {
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
        match segment {
            ParsedSegment::Prose(text) => {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(text);
            }
            ParsedSegment::Block { .. } => {}
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
