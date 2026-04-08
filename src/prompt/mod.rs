use crate::backend::{BackendCmd, CmdResponse, GetTextCmd, TerminalBackend, WindowId};
use crate::agent::AgentResponse;
use crate::types::{ParsedSegment, Session, Window, WindowState};

const SYSTEM_PROMPT: &str = "You are TAI, a terminal agent. You control terminal windows.

Format your response with code blocks:
- ```<window_id>:text\ncommand\n``` — send text to window stdin
- ```<window_id>:keys\nkey1 key2\n``` — send keypresses
- ```tai:cmd\nlaunch --title name -- command\n``` — TAI commands

Text outside blocks goes to chat.

When a process finishes (exit code shown), analyze the result and decide next steps.";

#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub system: String,
    pub dashboard: Vec<WindowSummary>,
    pub focused_windows: Vec<WindowView>,
    pub previous_response: Option<AgentResponse>,
    pub status: StatusInfo,
}

#[derive(Debug, Clone)]
pub struct WindowSummary {
    pub id: String,
    pub title: String,
    pub state_kind: WindowStateKind,
    pub backend_id: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowStateKind {
    Active,
    Frozen { exit_code: i32 },
    Archived,
}

#[derive(Debug, Clone)]
pub struct WindowView {
    pub id: String,
    pub title: String,
    pub content: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusInfo {
    pub active: usize,
    pub frozen: usize,
    pub focused: usize,
}

impl Prompt {
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut parts = Vec::new();

        parts.push(self.system.clone());
        parts.push(String::new());
        parts.push(self.render_dashboard());

        for w in &self.focused_windows {
            parts.push(Self::render_window(w));
        }

        if let Some(prev) = &self.previous_response {
            parts.push(String::new());
            parts.push("## Your previous response".to_string());
            parts.push(
                prev.segments
                    .iter()
                    .map(|s| match s {
                        ParsedSegment::Block {
                            window,
                            content,
                            mode,
                        } => {
                            format!("```{window}:{mode}\n{content}```")
                        }
                        ParsedSegment::Prose(t) => t.clone(),
                    })
                    .collect(),
            );
        }

        parts.push(String::new());
        parts.push(self.render_status_bar());

        parts.join("\n")
    }

    fn render_dashboard(&self) -> String {
        let mut lines = vec!["## Dashboard".to_string()];

        if self.dashboard.is_empty() {
            lines.push("No windows.".to_string());
        } else {
            for entry in &self.dashboard {
                let state = match entry.state_kind {
                    WindowStateKind::Active => {
                        let bid = entry.backend_id.as_deref().unwrap_or("?");
                        format!("[active] {} ({bid})", entry.title)
                    }
                    WindowStateKind::Frozen { exit_code } => {
                        format!("[frozen] exit code: {exit_code}")
                    }
                    WindowStateKind::Archived => "[archived]".to_string(),
                };
                let focus = if entry.focused { " *" } else { "" };
                lines.push(format!("- [{}] {state}{focus}", entry.id));
            }
        }

        lines.join("\n")
    }

    fn render_window(w: &WindowView) -> String {
        let exit_info = w
            .exit_code
            .map_or(String::new(), |c| format!("\n**Exit code: {c}**"));
        format!(
            "## Window [{}] {} (focused)\n{}{}\n```\n{}\n```",
            w.id,
            w.title,
            exit_info,
            if exit_info.is_empty() { "" } else { "\n" },
            w.content
        )
    }

    fn render_status_bar(&self) -> String {
        format!(
            "[STATUS] Active: {} | Frozen: {} | Focused: {}",
            self.status.active, self.status.frozen, self.status.focused
        )
    }
}

pub async fn build(
    session: &Session,
    backend: &dyn TerminalBackend,
    previous_response: Option<AgentResponse>,
) -> Result<Prompt, PromptError> {
    let dashboard = build_dashboard(session);
    let focused_windows = collect_focused_windows(session, backend).await?;
    let status = build_status(session);

    Ok(Prompt {
        system: SYSTEM_PROMPT.to_string(),
        dashboard,
        focused_windows,
        previous_response,
        status,
    })
}

fn build_dashboard(session: &Session) -> Vec<WindowSummary> {
    session
        .windows
        .iter()
        .map(|w| {
            let (state_kind, backend_id) = match &w.state {
                WindowState::Active {
                    backend_id,
                    pid: _,
                    title: _,
                } => (WindowStateKind::Active, Some(backend_id.clone())),
                WindowState::Frozen {
                    content: _,
                    exit_code,
                    captured_at: _,
                } => (
                    WindowStateKind::Frozen {
                        exit_code: *exit_code,
                    },
                    None,
                ),
                WindowState::Archived { .. } => (WindowStateKind::Archived, None),
            };
            let title = match &w.state {
                WindowState::Active { title, .. } => title.clone(),
                WindowState::Frozen { .. } | WindowState::Archived { .. } => String::new(),
            };
            WindowSummary {
                id: w.id.clone(),
                title,
                state_kind,
                backend_id,
                focused: w.focused,
            }
        })
        .collect()
}

async fn collect_focused_windows(
    session: &Session,
    backend: &dyn TerminalBackend,
) -> Result<Vec<WindowView>, PromptError> {
    let mut views = Vec::new();
    for window in session.focused_windows() {
        let content = get_window_content(window, backend).await?;
        let title = match &window.state {
            WindowState::Active { title, .. } => title.clone(),
            WindowState::Frozen { .. } | WindowState::Archived { .. } => String::new(),
        };
        views.push(WindowView {
            id: window.id.clone(),
            title,
            content,
            exit_code: window.state.exit_code(),
        });
    }
    Ok(views)
}

async fn get_window_content(
    window: &Window,
    backend: &dyn TerminalBackend,
) -> Result<String, PromptError> {
    match &window.state {
        WindowState::Active { backend_id, .. } => {
            let id = WindowId(backend_id.clone());
            match backend
                .execute(BackendCmd::Get(GetTextCmd { window: id }))
                .await
            {
                Ok(CmdResponse::Text(text)) => Ok(text),
                Ok(CmdResponse::Error(e)) => Err(PromptError::Backend(
                    crate::backend::BackendError::Communication(e),
                )),
                Ok(_) => Err(PromptError::Backend(
                    crate::backend::BackendError::Communication("unexpected response".into()),
                )),
                Err(e) => Err(PromptError::Backend(e)),
            }
        }
        WindowState::Frozen { content, .. } => Ok(content.clone()),
        WindowState::Archived { .. } => Ok("[archived]".to_string()),
    }
}

fn build_status(session: &Session) -> StatusInfo {
    StatusInfo {
        active: session.active_windows().count(),
        frozen: session
            .windows
            .iter()
            .filter(|w| w.state.is_frozen())
            .count(),
        focused: session.focused_windows().count(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("backend error: {0}")]
    Backend(#[from] crate::backend::BackendError),
}
