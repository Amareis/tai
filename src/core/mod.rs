use crate::agent::{Agent, AgentResponse};
use crate::backend::watch::Watcher;
use crate::backend::{
    BackendCmd, CloseCmd, CmdResponse, GetTextCmd, LaunchCmd, SendTextCmd, TerminalBackend, WindowId,
};
use crate::types::{BlockMode, ParsedSegment};
use std::collections::HashSet;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, info, trace, warn};

pub mod utils;

use crate::prompt::Prompt;
use crate::response::parse_response;

const CMD_WAIT_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("connection closed")]
    ConnectionClosed,

    #[error("readline error: {0}")]
    Readline(#[from] rustyline_async::ReadlineError),

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
        info!("run: starting server");
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
"
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
            self.tick().await?;

            if self.watcher.watch_entries().is_empty() {
                info!("run: no tracked windows left, exiting");
                break;
            }
        }

        Ok(())
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        info!("tick: start");
        self.rerun_watch_windows().await?;

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

    async fn execute_blocks(&mut self, segments: &[ParsedSegment]) -> Vec<ExecutedBlock> {
        let mut executed = Vec::new();
        let mut pending_ids: Vec<WindowId> = Vec::new();

        for segment in segments {
            if let ParsedSegment::Block {
                window,
                mode,
                content,
            } = segment
            {
                let block = self.execute_block_launch(window, mode, content).await;
                if let Some(ref id) = block.window_id {
                    pending_ids.push(id.clone());
                }
                executed.push(block);
            }
        }

        if !pending_ids.is_empty() {
            info!("execute_blocks: batch-waiting for {} windows", pending_ids.len());
            self.wait_for_windows(&pending_ids, CMD_WAIT_TIMEOUT).await;
        }

        executed
    }

    async fn execute_block_launch(
        &mut self,
        title: &str,
        mode: &BlockMode,
        content: &str,
    ) -> ExecutedBlock {
        match mode {
            BlockMode::Close => {
                let result = self.execute_close(title).await;
                ExecutedBlock {
                    title: title.to_string(),
                    window_id: None,
                    mode: *mode,
                    result: Some(result),
                }
            }
            BlockMode::Text => match self.launch_and_send(title, content).await {
                Ok(id) => ExecutedBlock {
                    title: title.to_string(),
                    window_id: Some(id),
                    mode: *mode,
                    result: None,
                },
                Err(e) => ExecutedBlock {
                    title: title.to_string(),
                    window_id: None,
                    mode: *mode,
                    result: Some(format!("[{title}] Error: {e}")),
                },
            },
            BlockMode::Write => {
                let result = Self::execute_file_write(title, content);
                let ok = !result.contains("Error:");
                let window_id = if ok {
                    let view_title = std::path::Path::new(title)
                        .file_name()
                        .map_or(title.to_string(), |n| n.to_string_lossy().to_string());
                    self.launch_and_send(&view_title, &format!("cat -n {title}")).await.ok()
                } else {
                    None
                };
                ExecutedBlock {
                    title: title.to_string(),
                    window_id,
                    mode: *mode,
                    result: Some(result),
                }
            }
        }
    }

    async fn launch_and_send(&mut self, title: &str, content: &str) -> Result<WindowId, String> {
        info!("launch_and_send: '{title}'");
        let launch_cmd = LaunchCmd {
            title: Some(title.to_string()),
            command: String::new(),
        };

        let id = match self.back.cmd_launch(&launch_cmd).await {
            Ok(id) => {
                info!("launch_and_send: shell launched id={id} for '{title}'");
                id
            }
            Err(e) => {
                return Err(format!("launch failed: {e}"));
            }
        };

        let send_cmd = SendTextCmd {
            window: id.clone(),
            text: vec![format!("{content}\n")],
        };
        if let Err(e) = self.back.execute(BackendCmd::Send(send_cmd)).await {
            return Err(format!("send-text failed: {e}"));
        }

        self.watcher
            .track(id.clone(), title.to_string(), content.to_string());

        Ok(id)
    }

    async fn wait_for_windows(&mut self, ids: &[WindowId], timeout: Duration) {
        trace!("wait_for_windows: waiting for {} windows", ids.len());
        let mut pending: HashSet<WindowId> = ids.iter().cloned().collect();
        let start = std::time::Instant::now();
        let mut poll_count = 0u32;

        while !pending.is_empty() {
            if start.elapsed() > timeout {
                warn!(
                    "wait_for_windows: timeout after {poll_count} polls, {} still pending",
                    pending.len()
                );
                break;
            }
            poll_count += 1;

            if let Ok(CmdResponse::Windows(windows)) = self.back.execute(BackendCmd::List).await {
                for w in &windows {
                    if w.is_at_prompt {
                        pending.remove(&w.id);
                    }
                }
            }

            if !pending.is_empty() {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }

        info!(
            "wait_for_windows: done ({poll_count} polls, {} pending)",
            pending.len()
        );
    }

    async fn rerun_watch_windows(&mut self) -> Result<(), CoreError> {
        let entries = self.watcher.watch_entries();
        if entries.is_empty() {
            debug!("rerun_watch_windows: no tracked windows");
            return Ok(());
        }
        let count = entries.len();
        info!("rerun_watch_windows: {count} windows");

        for (id, title, command) in &entries {
            debug!("rerun_watch_windows: sending to '{title}' (id={id})");
            let send_cmd = SendTextCmd {
                window: id.clone(),
                text: vec![format!("reset && {command}\n")],
            };
            if let Err(e) = self.back.execute(BackendCmd::Send(send_cmd)).await {
                warn!("rerun_watch_windows: send-text error for '{title}' (id={id}): {e}");
            }
        }

        let ids: Vec<WindowId> = entries.iter().map(|(id, _, _)| id.clone()).collect();
        self.wait_for_windows(&ids, CMD_WAIT_TIMEOUT).await;

        Ok(())
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

    async fn execute_close(&mut self, title: &str) -> String {
        if let Some(id) = self.get_id_for_title(title).await {
            info!("execute_close: closing '{title}' (id={id})");
            let _ = self
                .back
                .execute(BackendCmd::Close(CloseCmd {
                    window_id: id.clone(),
                }))
                .await;
            self.watcher.remove_by_title(title);
            format!("[{title}] closed (id={id})")
        } else {
            warn!("execute_close: window '{title}' not found");
            format!("[{title}] Error: window not found")
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