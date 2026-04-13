use crate::agent::{Agent, AgentResponse};
use crate::backend::Backend;
use crate::prompt::{Prompt, TrackedView};
use crate::types::{BlockMode, ParsedSegment};
use std::fmt::Write;
use thiserror::Error;
use tracing::{debug, info};

use crate::response::parse_response;

struct TrackedCmd {
    title: String,
    command: String,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("agent error: {0}")]
    Agent(#[from] crate::agent::AgentError),
}

struct ExecutedBlock {
    title: String,
    output: Option<String>,
}

pub struct Server {
    back: Box<dyn Backend>,
    pub agent: Box<dyn Agent>,
    tracked: Vec<TrackedCmd>,
    pub debug: bool,
    last_tick: Option<(AgentResponse, String)>,
}

impl Server {
    #[must_use]
    pub fn new(back: Box<dyn Backend>, agent: Box<dyn Agent>) -> Self {
        Self {
            back,
            agent,
            tracked: Vec::new(),
            debug: false,
            last_tick: None,
        }
    }

    #[must_use]
    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        info!("run: starting server");

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
        let feedback = Self::collect_feedback(&executed);
        info!("run: initial feedback collected ({} chars)", feedback.len());

        self.last_tick = Some((
            AgentResponse {
                reasoning: String::new(),
                segments,
            },
            feedback,
        ));

        info!("run: entering tick loop");
        loop {
            self.tick().await?;

            if self.tracked.is_empty() {
                info!("run: no tracked commands left, exiting");
                break;
            }
        }

        Ok(())
    }

    pub async fn tick(&mut self) -> Result<(), CoreError> {
        info!("tick: start");

        let tracked_views = self.rerun_tracked().await;

        let (prev_response, prev_feedback) = match self.last_tick.take() {
            Some((resp, fb)) => (Some(resp), Some(fb)),
            None => (None, None),
        };

        let prompt = Prompt::build(tracked_views, prev_response, prev_feedback);
        info!("tick: prompt built, calling agent");

        let response = self.agent.step(&prompt).await?;
        info!("tick: agent responded ({} segments)", response.segments.len());

        let executed = self.execute_blocks(&response.segments).await;
        info!("tick: {} blocks executed", executed.len());

        let feedback = Self::collect_feedback(&executed);
        info!("tick: feedback collected ({} chars)", feedback.len());

        self.last_tick = Some((response.clone(), feedback));

        info!("tick: done");
        Ok(())
    }

    async fn rerun_tracked(&mut self) -> Vec<TrackedView> {
        if self.tracked.is_empty() {
            return Vec::new();
        }

        let mut views = Vec::with_capacity(self.tracked.len());
        for tc in &self.tracked {
            debug!("rerun_tracked: '{}' cmd={}", tc.title, tc.command);
            let output = self.back.run(&tc.title, &tc.command).await;
            views.push(TrackedView {
                title: tc.title.clone(),
                output: output.stdout,
                exit_code: output.exit_code,
            });
        }

        views
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
                self.tracked.retain(|tc| tc.title != title);
                info!("execute_block: '{title}' closed");
                ExecutedBlock {
                    title: title.to_string(),
                    output: None,
                }
            }
            BlockMode::Text => {
                let output = self.back.run(title, content).await;
                self.tracked.push(TrackedCmd {
                    title: title.to_string(),
                    command: content.to_string(),
                });
                ExecutedBlock {
                    title: title.to_string(),
                    output: Some(output.stdout),
                }
            }
            BlockMode::Write => {
                let result = Self::execute_file_write(title, content);
                if !result.contains("Error:") {
                    let view_title = std::path::Path::new(title)
                        .file_name()
                        .map_or(title.to_string(), |n| n.to_string_lossy().to_string());
                    let view_cmd = format!("cat -n {title}");
                    let _ = self.back.run(&view_title, &view_cmd).await;
                    self.tracked.push(TrackedCmd {
                        title: view_title,
                        command: view_cmd,
                    });
                }
                ExecutedBlock {
                    title: title.to_string(),
                    output: Some(result),
                }
            }
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

    fn collect_feedback(executed: &[ExecutedBlock]) -> String {
        let mut feedback = String::new();
        for block in executed {
            if !feedback.is_empty() {
                feedback.push('\n');
            }
            match &block.output {
                Some(output) => {
                    let trimmed = output.trim();
                    if trimmed.is_empty() {
                        let _ = write!(feedback, "[{}]", block.title);
                    } else {
                        let _ = write!(feedback, "[{}]\n{}", block.title, trimmed);
                    }
                }
                None => {
                    let _ = write!(feedback, "[{}] closed", block.title);
                }
            }
        }
        feedback
    }
}
