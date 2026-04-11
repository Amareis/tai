use crate::agent::{Agent, AgentResponse};
use crate::backend::watch::Watcher;
use crate::backend::{
    BackendCmd, CloseCmd, CmdResponse, GetTextCmd, LaunchCmd, SendTextCmd, TerminalBackend, WindowId,
};
use crate::types::{BlockMode, ParsedSegment, TickTrigger};
use rustyline_async::ReadlineError;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::select;
use tracing::{info, warn, debug, trace};

pub mod utils;

use crate::prompt::Prompt;
use crate::response::parse_response;
use utils::sleep_some_or_forever;

const TRIGGER_TIMEOUT: Duration = Duration::from_secs(1);
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
        info!("run: starting server main loop");
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        info!("run: executing initial windows");
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
        info!("run: initial feedback collected ({} chars)", feedback.len());

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

        info!("run: entering tick loop");
        loop {
            if self.debug {
                let _ = lines.next_line().await;
            }
            info!("run: waiting for trigger (timeout {:?})", TRIGGER_TIMEOUT);
            let _events = self.wait_trigger(Some(TRIGGER_TIMEOUT)).await?;
            info!("run: trigger fired, starting tick");
            self.tick().await?;
            info!("run: tick completed");
        }
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        info!("tick: rerunning watch windows");
        self.rerun_watch_windows().await?;
        info!("tick: watch windows rerun done, building prompt");

        let (prev_response, prev_feedback) = match self.last_tick.take() {
            Some((resp, fb)) => (Some(resp), Some(fb)),
            None => (None, None),
        };

        let prompt = Prompt::build(self.back.as_ref(), prev_response, prev_feedback).await?;
        info!("tick: prompt built, calling agent");

        let response = self.agent.step(&prompt).await?;
        info!("tick: agent responded ({} segments)", response.segments.len());

        let executed = self.execute_blocks(&response.segments).await;
        info!("tick: {} blocks executed", executed.len());

        let feedback = self.collect_feedback(executed).await;
        info!("tick: feedback collected ({} chars)", feedback.len());

        self.last_tick = Some((response.clone(), feedback));

        info!("tick: done");
        Ok(())
    }

    pub async fn wait_trigger(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<TickTrigger, CoreError> {
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
                let window_id = self.execute_text(title, content).await;
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

    async fn execute_text(&mut self, title: &str, content: &str) -> WindowId {
        info!("execute_text: launching shell for '{title}'");
        let launch_cmd = LaunchCmd {
            title: Some(title.to_string()),
            command: String::new(),
        };

        let id = match self.back.cmd_launch(&launch_cmd).await {
            Ok(id) => {
                info!("execute_text: shell launched id={id} for '{title}'");
                id
            }
            Err(e) => {
                warn!("execute_text: launch error for '{title}': {e}");
                return WindowId::new("error");
            }
        };

        debug!("execute_text: sending command to '{title}' (id={id})");
        let send_cmd = SendTextCmd {
            window: id.clone(),
            text: vec![format!("{content}\n")],
        };
        if let Err(e) = self.back.execute(BackendCmd::Send(send_cmd)).await {
            warn!("execute_text: send-text error for '{title}': {e}");
        }

        self.watcher
            .track(id.clone(), title.to_string(), content.to_string());

        info!("execute_text: waiting for '{title}' (id={id})");
        self.wait_for_window(&id, CMD_WAIT_TIMEOUT).await;
        info!("execute_text: done '{title}' (id={id})");

        id
    }

    async fn rerun_watch_windows(&mut self) -> Result<(), CoreError> {
        let entries = self.watcher.watch_entries();
        let count = entries.len();
        if count == 0 {
            debug!("rerun_watch_windows: no tracked windows");
            return Ok(());
        }
        info!("rerun_watch_windows: rerunning {count} windows");
        for (id, title, command) in entries {
            debug!("rerun_watch_windows: sending to '{title}' (id={id})");
            let send_cmd = SendTextCmd {
                window: id.clone(),
                text: vec![format!("clear && {command}\n")],
            };
            if let Err(e) = self.back.execute(BackendCmd::Send(send_cmd)).await {
                warn!("rerun_watch_windows: send-text error for '{title}' (id={id}): {e}");
                continue;
            }

            self.wait_for_window(&id, CMD_WAIT_TIMEOUT).await;
        }
        info!("rerun_watch_windows: done");
        Ok(())
    }

    async fn wait_for_window(&mut self, id: &WindowId, timeout: Duration) {
        trace!("wait_for_window: waiting for {id} (timeout {timeout:?})");
        let start = std::time::Instant::now();
        let mut attempts = 0u32;
        loop {
            if start.elapsed() > timeout {
                warn!("wait_for_window: timeout for {id} after {attempts} polls");
                return;
            }

            if let Ok(CmdResponse::Windows(windows)) = self.back.execute(BackendCmd::List).await
                && let Some(w) = windows.iter().find(|w| &w.id == id)
                && w.is_at_prompt
            {
                trace!("wait_for_window: {id} at_prompt after {} polls", attempts + 1);
                return;
            }

            attempts += 1;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

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
            info!("execute_close: closing '{title}' (id={id})");
            let _ = self
                .back
                .execute(BackendCmd::Close(CloseCmd {
                    window_id: id.clone(),
                }))
                .await;
            self.watcher.remove_by_title(title);
            Some(id)
        } else {
            warn!("execute_close: window '{title}' not found");
            None
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

    fn execute_file_write(path: &str, content: &str) -> String {
        info!("execute_file_write: writing {} bytes to '{path}'", content.len());
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