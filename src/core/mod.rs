use crate::agent::{Agent, AgentResponse};
use crate::backend::Backend;
use crate::state::{State, TrackedView};
use crate::types::{BlockMode, ParsedBlock};
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

    pub async fn run(&mut self, initial: Option<AgentResponse>) -> Result<(), CoreError> {
        info!("run: starting server");
        std::fs::create_dir_all(&self.debug_dir).ok();

        if let Some(resp) = initial {
            self.apply_segments_sorted(&resp.segments);
            self.last_tick = Some(resp);
        }

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

        let pending = self.last_tick.as_ref().map(|r| r.segments.clone()).unwrap_or_default();
        self.debug_write_segments("pending", &pending);
        self.debug_wait("before apply").await;

        self.apply_segments_sorted(&pending);

        let tracked_views = self.run_tracked().await;
        self.debug_write_views(&tracked_views);

        let prev_response = self.last_tick.take();
        let state = State::build(tracked_views, prev_response, self.tick_n);
        info!("tick #{}: state built, calling agent", self.tick_n);

        self.debug_wait("before agent call").await;

        let response = self.agent.step(&state).await?;
        info!("tick #{}: agent responded ({} segments)", self.tick_n, response.segments.len());

        self.debug_write_response(&response);
        self.last_tick = Some(response);

        info!("tick #{}: done", self.tick_n);
        Ok(())
    }

    fn apply_segments_sorted(&mut self, segments: &[ParsedBlock]) {
        let mut sorted: Vec<&ParsedBlock> = segments.iter().collect();
        sorted.sort_by_key(|b| match b.mode {
            BlockMode::Close => 0,
            BlockMode::Exec => 1,
            BlockMode::View => 2,
        });
        for block in &sorted {
            match block.mode {
                BlockMode::Close => {
                    self.tracked.retain(|tc| tc.title != block.window);
                    info!("apply: '{}' closed", block.window);
                }
                BlockMode::Exec => {
                    self.upsert_tracked(block.window.clone(), block.content.clone(), false);
                    info!("apply: '{}' tracked (exec)", block.window);
                }
                BlockMode::View => {
                    self.upsert_tracked(block.window.clone(), block.content.clone(), true);
                    info!("apply: '{}' tracked (view)", block.window);
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

        self.tracked.sort_by(|a, b| {
            let order = |rerun: bool| i32::from(rerun);
            order(a.rerun).cmp(&order(b.rerun))
        });

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
                rerun: tc.rerun,
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

    fn debug_write_segments(&self, prefix: &str, segments: &[ParsedBlock]) {
        for block in segments {
            let filename = format!("{prefix}-{window}", window = block.window);
            let path = self.debug_dir.join(&filename);
            let mut content = String::new();
            let _ = writeln!(content, "# mode={}", block.mode);
            content.push_str(&block.content);
            std::fs::write(&path, content).ok();
        }
    }

    fn debug_write_response(&self, response: &AgentResponse) {
        for block in &response.segments {
            let path = self.debug_dir.join(&block.window);
            let mut content = String::new();
            let _ = writeln!(content, "# mode={}", block.mode);
            content.push_str(&block.content);
            std::fs::write(&path, content).ok();
        }
    }
}
