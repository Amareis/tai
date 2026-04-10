use std::fmt::Write;
use std::time::Duration;
use crate::agent::Agent;
use crate::agent::AgentResponse;
use crate::backend::watch::Watcher;
use crate::backend::{BackendCmd, CloseCmd, CmdResponse, LaunchCmd, SetTitleCmd, TerminalBackend, WindowId};
use crate::types::{BlockMode, ParsedSegment, TickTrigger};
use rustyline_async::ReadlineError;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::select;
use tracing::info;

pub mod utils;

use crate::prompt::Prompt;
use utils::sleep_some_or_forever;

const TRIGGER_TIMEOUT: Duration = Duration::from_secs(5);

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
    back: Box<dyn TerminalBackend>,
    pub agent: Box<dyn Agent>,
    watcher: Watcher,
    pub debug: bool,
    last_tick: Option<(AgentResponse, String)>,
}

impl Server {
    #[must_use]
    pub fn new(back: Box<dyn TerminalBackend>, agent: Box<dyn Agent>) -> Self {
        Self {
            back,
            agent,
            watcher: Watcher::new(),
            debug: false,
            last_tick: None,
        }
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        // 1. Создаем асинхронный stdin
        let stdin = tokio::io::stdin();

        // 2. Обязательно оборачиваем в BufReader для построчного чтения
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        let _ = self.back.execute(BackendCmd::Title(SetTitleCmd{
            window_id: WindowId("1".to_string()),
            title: vec!["task".to_string()]
        })).await;
        self.run_cmd("task", "watch -t cat TASK.md").await;
        self.run_cmd("tree", "watch -t tree --gitignore").await;

        loop {
            if self.debug {
                let _ = lines.next_line().await;
            }
            let _events = self.wait_trigger(Some(TRIGGER_TIMEOUT)).await?;
            self.tick().await?;
        }
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        let (prev_response, prev_feedback) = match self.last_tick.take() {
            Some((resp, fb)) => (Some(resp), Some(fb)),
            None => (None, None),
        };

        let prompt = Prompt::build(self.back.as_ref(), prev_response, prev_feedback).await?;

        let response = self.agent.step(&prompt).await?;

        let results = self.execute_blocks(&response.segments).await;

        let feedback = format_results(&response.segments, &results);

        self.last_tick = Some((response.clone(), feedback));

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
            () = sleep_some_or_forever(timeout) => {
                Ok(TickTrigger::IdleTimeout)
            },
        }
    }

    async fn run_cmd(&mut self, window: &str, content: &str) {
        let seg = vec![ParsedSegment::Block {
            window: window.to_string(),
            mode: BlockMode::Text,
            content: content.to_string(),
        }];
        let _ = self.execute_blocks(&seg).await;
    }

    async fn execute_blocks(&mut self, segments: &[ParsedSegment]) -> Vec<BlockResult> {
        let mut results = Vec::new();

        for segment in segments {
            if let ParsedSegment::Block { window, mode, content } = segment {
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
            BlockMode::Close => self.execute_close(window).await,
            BlockMode::Text => self.execute_window_block(window, content).await,
            BlockMode::Write => Self::execute_file_write(window, content),
        }
    }

    async fn execute_close(&mut self, title: &str) -> BlockResult {
        if let Some(id) = self.get_id_for_title(title).await {
            let cmd = BackendCmd::Close(CloseCmd {
                window_id: id,
            });
            self.execute_backend_cmd(cmd, title).await
        } else {
            BlockResult::Error(title.to_string(), format!("window '{title}' not found"))
        }
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
            let launch_cmd = BackendCmd::Launch(LaunchCmd {
                title: Some(title.to_string()),
                command: vec!["zsh".to_string()],
            });
            match self.back.execute(launch_cmd).await {
                Ok(CmdResponse::WindowCreated(id)) => {
                    self.watcher.track(id.clone(), true);
                    id
                }
                Ok(CmdResponse::Error(e)) => {
                    return BlockResult::Error(title.to_string(), format!("launch failed: {e}"));
                }
                Ok(_) => {
                    return BlockResult::Error(title.to_string(), "unexpected response from launch".to_string());
                }
                Err(e) => {
                    return BlockResult::Error(title.to_string(), format!("launch failed: {e}"));
                }
            }
        };

        let text_to_send = format!("{content}\n");
        let cmd = BackendCmd::Send(crate::backend::SendTextCmd {
            window: id,
            text: vec![text_to_send],
        });
        self.execute_backend_cmd(cmd, title).await
    }

    fn execute_file_write(path: &str, content: &str) -> BlockResult {
        let file_path = std::path::Path::new(path);
        if let Some(parent) = file_path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return BlockResult::Error(path.to_string(), format!("mkdir failed: {e}"));
        }
        match std::fs::write(path, content) {
            Ok(()) => {
                let bytes = content.len();
                BlockResult::Ok(format!("{path} ({bytes} bytes written)"))
            }
            Err(e) => BlockResult::Error(path.to_string(), format!("write failed: {e}")),
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