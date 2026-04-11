use crate::agent::{Agent, AgentResponse};
use crate::backend::watch::Watcher;
use crate::backend::{
    BackendCmd, CloseCmd, CmdResponse, GetTextCmd, LaunchCmd, TerminalBackend, WindowId,
};
use crate::types::{BlockMode, ParsedSegment, TickTrigger};
use rustyline_async::ReadlineError;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::select;
use tracing::info;

pub mod utils;

use crate::prompt::Prompt;
use crate::response::parse_response;
use utils::sleep_some_or_forever;

const TRIGGER_TIMEOUT: Duration = Duration::from_secs(30);
const CMD_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

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

struct ExecutedBlock {
    title: String,
    window_id: Option<WindowId>,
    #[allow(dead_code)]
    mode: BlockMode,
    /// Pre-computed result string (for Write and Close)
    result: Option<String>,
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
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        let segments = parse_response(
            r"Current task
```task
cat TASK.md
```
Current dir and contents
```tree
pwd && tree --gitignore
```
",
        );
        let executed = self.execute_blocks(&segments).await;

        let feedback = self.collect_feedback(executed).await;

        self.last_tick = Some((
            AgentResponse {
                reasoning: String::new(),
                segments,
            },
            feedback,
        ));

        let _ = self
            .back
            .execute(BackendCmd::Close(CloseCmd {
                window_id: WindowId("1".to_string()),
            }))
            .await;

        loop {
            if self.debug {
                let _ = lines.next_line().await;
            }
            let _events = self.wait_trigger(Some(TRIGGER_TIMEOUT)).await?;
            self.tick().await?;
        }
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        self.refresh_watch_windows().await?;

        let (prev_response, prev_feedback) = match self.last_tick.take() {
            Some((resp, fb)) => (Some(resp), Some(fb)),
            None => (None, None),
        };

        let prompt = Prompt::build(self.back.as_ref(), prev_response, prev_feedback).await?;

        let response = self.agent.step(&prompt).await?;

        let executed = self.execute_blocks(&response.segments).await;

        let feedback = self.collect_feedback(executed).await;

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

    async fn execute_blocks(&mut self, segments: &[ParsedSegment]) -> Vec<ExecutedBlock> {
        let mut executed = Vec::new();

        for segment in segments {
            if let ParsedSegment::Block {
                window,
                mode,
                content,
            } = segment
            {
                let result = self.execute_block(window, mode, content).await;
                executed.push(result);
            }
        }

        executed
    }

    async fn execute_block(
        &mut self,
        title: &str,
        mode: &BlockMode,
        content: &str,
    ) -> ExecutedBlock {
        match mode {
            BlockMode::Close => {
                let _window_id = self.execute_close(title).await;
                ExecutedBlock {
                    title: title.to_string(),
                    window_id: None,
                    mode: *mode,
                    result: Some(format!("[{title}] closed")),
                }
            }
            BlockMode::Text => {
                let window_id = self.execute_one_shot(title, content).await;
                ExecutedBlock {
                    title: title.to_string(),
                    window_id: Some(window_id),
                    mode: *mode,
                    result: None,
                }
            }
            BlockMode::Write => {
                let result = Self::execute_file_write(title, content);
                ExecutedBlock {
                    title: title.to_string(),
                    window_id: None,
                    mode: *mode,
                    result: Some(result),
                }
            }
        }
    }

    async fn execute_one_shot(&mut self, title: &str, content: &str) -> WindowId {
        let launch_cmd = LaunchCmd {
            title: Some(title.to_string()),
            command: content.to_string(),
        };

        let id = match self.back.cmd_launch(&launch_cmd).await {
            Ok(id) => {
                self.watcher
                    .track(id.clone(), title.to_string(), content.to_string());
                id
            }
            Err(e) => {
                info!("launch error for '{title}': {e}");
                return WindowId::new("error");
            }
        };

        // Wait for command to finish (at_prompt) with timeout
        self.wait_for_window(&id, CMD_WAIT_TIMEOUT).await;

        id
    }

    /// Poll until window reaches `at_prompt` or timeout.
    async fn wait_for_window(&mut self, id: &WindowId, timeout: Duration) {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                info!("wait_for_window: timeout for {id}");
                return;
            }

            if let Ok(CmdResponse::Windows(windows)) = self.back.execute(BackendCmd::List).await
                && let Some(w) = windows.iter().find(|w| &w.id == id)
                && w.is_at_prompt
            {
                return;
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// After executing blocks, read window output for one-shot commands.
    async fn collect_feedback(&mut self, executed: Vec<ExecutedBlock>) -> String {
        let mut feedback = String::new();

        for block in &executed {
            let output = if let Some(ref result) = block.result {
                result.clone()
            } else if let Some(ref id) = block.window_id {
                self.read_window_output(id, &block.title).await
            } else {
                format!("[{}]", block.title)
            };

            if !feedback.is_empty() {
                feedback.push('\n');
            }
            feedback.push_str(&output);
        }

        feedback
    }

    /// Read window content + exit code for feedback.
    async fn read_window_output(&mut self, id: &WindowId, title: &str) -> String {
        let text = match self
            .back
            .execute(BackendCmd::Get(GetTextCmd {
                window_id: id.clone(),
            }))
            .await
        {
            Ok(CmdResponse::Text(t)) => t,
            _ => String::new(),
        };

        let exit_code = match self.back.execute(BackendCmd::List).await {
            Ok(CmdResponse::Windows(windows)) => windows
                .iter()
                .find(|w| &w.id == id)
                .and_then(|w| w.last_cmd_exit_status),
            _ => None,
        };

        let exit_info = match exit_code {
            Some(code) => format!("exit {code}"),
            None => String::new(),
        };

        let trimmed = text.trim();
        if trimmed.is_empty() {
            format!("[{title}] {exit_info}")
        } else {
            format!("[{title}] {exit_info}\n{trimmed}")
        }
    }

    async fn execute_close(&mut self, title: &str) -> Option<WindowId> {
        if let Some(id) = self.get_id_for_title(title).await {
            let _ = self
                .back
                .execute(BackendCmd::Close(CloseCmd {
                    window_id: id.clone(),
                }))
                .await;
            self.watcher.remove_by_title(title);
            Some(id)
        } else {
            None
        }
    }

    async fn refresh_watch_windows(&mut self) -> Result<(), CoreError> {
        let entries = self.watcher.watch_entries();
        for (old_id, title, command) in entries {
            let _ = self
                .back
                .execute(BackendCmd::Close(CloseCmd {
                    window_id: old_id.clone(),
                }))
                .await;

            let launch_cmd = LaunchCmd {
                title: Some(title.clone()),
                command: command.clone(),
            };

            let new_id = match self.back.cmd_launch(&launch_cmd).await {
                Ok(id) => id,
                Err(e) => {
                    info!("refresh: launch error for '{title}': {e}");
                    continue;
                }
            };

            self.wait_for_window(&new_id, CMD_WAIT_TIMEOUT).await;

            self.watcher.update_id(&old_id, new_id);
        }
        Ok(())
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

    fn execute_file_write(path: &str, content: &str) -> String {
        let file_path = std::path::Path::new(path);
        if let Some(parent) = file_path.parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return format!("[{path}] Error: mkdir failed: {e}");
        }
        match std::fs::write(path, content) {
            Ok(()) => {
                let bytes = content.len();
                format!("[{path}] {bytes} bytes written")
            }
            Err(e) => format!("[{path}] Error: write failed: {e}"),
        }
    }
}
