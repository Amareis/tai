use crate::agent::{Agent, AgentResponse};
use crate::backend::Backend;
use crate::prompt::{Prompt, TrackedView};
use crate::types::{BlockMode, ParsedSegment};
use std::fmt::Write;
use std::path::PathBuf;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, info};

struct TrackedCmd {
    title: String,
    command: String,
    rerun: bool,
    cached_output: Option<String>,
    cached_exit: Option<i32>,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("agent error: {0}")]
    Agent(#[from] crate::agent::AgentError),
}

pub struct Server {
    back: Box<dyn Backend>,
    pub agent: Box<dyn Agent>,
    tracked: Vec<TrackedCmd>,
    pending: Vec<ParsedSegment>,
    pub debug: bool,
    tick_n: u64,
    last_tick: Option<AgentResponse>,
    debug_dir: PathBuf,
}

impl Server {
    #[must_use]
    pub fn new(back: Box<dyn Backend>, agent: Box<dyn Agent>) -> Self {
        Self {
            back,
            agent,
            tracked: Vec::new(),
            pending: Vec::new(),
            debug: false,
            tick_n: 0,
            last_tick: None,
            debug_dir: PathBuf::from("tai-debug"),
        }
    }

    #[must_use]
    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }

    pub async fn run(&mut self, initial: &[(&str, &str)]) -> Result<(), CoreError> {
        info!("run: starting server");
        std::fs::create_dir_all(&self.debug_dir).ok();

        let segments: Vec<ParsedSegment> = initial
            .iter()
            .map(|(title, cmd)| ParsedSegment::Block {
                window: title.to_string(),
                mode: BlockMode::View,
                content: cmd.to_string(),
            })
            .collect();
        self.apply_segments(&segments);
        self.last_tick = Some(AgentResponse {
            reasoning: String::new(),
            segments,
        });

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
        self.tick_n += 1;
        info!("tick #{}: start", self.tick_n);

        self.debug_write_pending();
        self.debug_wait("before apply_pending").await;

        self.apply_pending();

        let tracked_views = self.run_tracked().await;
        self.debug_write_views(&tracked_views);

        let prev_response = self.last_tick.take();
        let prompt = Prompt::build(tracked_views, prev_response);
        info!("tick #{}: prompt built, calling agent", self.tick_n);

        self.debug_wait("before agent call").await;

        let response = self.agent.step(&prompt).await?;
        info!("tick #{}: agent responded ({} segments)", self.tick_n, response.segments.len());

        self.debug_write_response(&response);

        self.pending.clone_from(&response.segments);
        self.last_tick = Some(response);

        info!("tick #{}: done", self.tick_n);
        Ok(())
    }

    fn apply_pending(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        self.apply_segments(&pending);
    }

    fn apply_segments(&mut self, segments: &[ParsedSegment]) {
        for segment in segments {
            if let ParsedSegment::Block {
                window,
                mode,
                content,
            } = segment
            {
                match mode {
                    BlockMode::Close => {
                        self.tracked.retain(|tc| tc.title != *window);
                        info!("apply: '{window}' closed");
                    }
                    BlockMode::View => {
                        self.upsert_tracked(window.clone(), content.clone(), true);
                        info!("apply: '{window}' tracked (view)");
                    }
                    BlockMode::Exec => {
                        self.upsert_tracked(window.clone(), content.clone(), false);
                        info!("apply: '{window}' tracked (exec)");
                    }
                }
            }
        }
    }

    fn upsert_tracked(&mut self, title: String, command: String, rerun: bool) {
        if let Some(existing) = self.tracked.iter_mut().find(|tc| tc.title == title) {
            existing.command = command;
            existing.rerun = rerun;
            existing.cached_output = None;
            existing.cached_exit = None;
        } else {
            self.tracked.push(TrackedCmd {
                title,
                command,
                rerun,
                cached_output: None,
                cached_exit: None,
            });
        }
    }

    async fn run_tracked(&mut self) -> Vec<TrackedView> {
        if self.tracked.is_empty() {
            return Vec::new();
        }

        let mut views = Vec::with_capacity(self.tracked.len());
        for tc in &mut self.tracked {
            if tc.rerun || tc.cached_output.is_none() {
                debug!("run_tracked: '{}' executing cmd={}", tc.title, tc.command);
                let output = self.back.run(&tc.title, &tc.command).await;
                tc.cached_output = Some(output.stdout);
                tc.cached_exit = Some(output.exit_code);
            } else {
                debug!("run_tracked: '{}' using cached output", tc.title);
            }

            views.push(TrackedView {
                title: tc.title.clone(),
                output: tc.cached_output.clone().unwrap_or_default(),
                exit_code: tc.cached_exit.unwrap_or(0),
            });
        }

        views
    }

    async fn debug_wait(&self, point: &str) {
        if !self.debug {
            return;
        }
        info!("debug: press Enter to continue ({point})...");
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        let _ = lines.next_line().await;
    }

    fn debug_write_views(&self, views: &[TrackedView]) {
        for v in views {
            let path = self.debug_dir.join(&v.title);
            let mut content = String::new();
            let _ = writeln!(content, "# exit {}", v.exit_code);
            content.push_str(&v.output);
            std::fs::write(&path, content).ok();
        }
    }

    fn debug_write_pending(&self) {
        for seg in &self.pending {
            if let ParsedSegment::Block { window, mode, content: block_content } = seg {
                let filename = format!("pending-{window}");
                let path = self.debug_dir.join(&filename);
                let mut content = String::new();
                let _ = writeln!(content, "# mode={mode}");
                content.push_str(block_content);
                std::fs::write(&path, content).ok();
            }
        }
    }

    fn debug_write_response(&self, response: &AgentResponse) {
        for seg in &response.segments {
            if let ParsedSegment::Block { window, mode, content: block_content } = seg {
                let path = self.debug_dir.join(window);
                let mut content = String::new();
                let _ = writeln!(content, "# mode={mode}");
                content.push_str(block_content);
                std::fs::write(&path, content).ok();
            }
        }
    }
}
