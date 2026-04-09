use crate::agent::AgentResponse;
use crate::backend::{
    BackendCmd, BackendError, CmdResponse, GetTextCmd, Terminal, TerminalBackend, WindowId,
};
use crate::types::ParsedSegment;

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
    pub dashboard: Vec<Terminal>,
    pub focused_windows: Vec<WindowView>,
    pub previous_response: Option<AgentResponse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowStateSummary {
    Active,
    Frozen { exit_code: i32 },
    Archived,
}

#[derive(Debug, Clone)]
pub struct WindowView {
    pub id: WindowId,
    pub title: String,
    pub content: String,
    pub exit_code: Option<i32>,
}

impl Prompt {
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut parts = Vec::new();

        parts.push(self.system.clone());
        parts.push(String::new());

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

        parts.push(self.render_dashboard());

        parts.push(String::new());

        parts.join("\n")
    }

    fn render_dashboard(&self) -> String {
        let mut lines = vec!["## Dashboard".to_string()];

        if self.dashboard.is_empty() {
            lines.push("No windows.".to_string());
        } else {
            lines.push(format!("Oened {} teminals: ", {self.dashboard.len()}));
            lines.extend(self.dashboard.iter().map(|w| {
                format!(
                    "{} | {} | pid {} | prompt: {}",
                    w.id, w.title, w.pid, w.is_at_prompt
                )
            }));
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
}

pub async fn build(
    backend: &dyn TerminalBackend,
    previous_response: Option<AgentResponse>,
) -> Result<Prompt, PromptError> {
    let CmdResponse::Windows(list) = backend.execute(BackendCmd::List).await? else {
        return Err(BackendError::Communication("unexpected response".into()).into());
    };

    let dashboard = build_dashboard(&list);
    let focused_windows = collect_windows(backend, &list).await?;

    Ok(Prompt {
        system: SYSTEM_PROMPT.to_string(),
        dashboard,
        focused_windows,
        previous_response,
    })
}

fn build_dashboard(list: &[Terminal]) -> Vec<Terminal> {
    Vec::from(list)
}

async fn collect_windows(backend: &dyn TerminalBackend, list: &Vec<Terminal>) -> Result<Vec<WindowView>, PromptError> {
    let mut views = Vec::new();
    for window in list {
        let content = get_window_content(window, backend).await?;
        views.push(WindowView {
            id: window.id.clone(),
            title: window.title.clone(),
            content,
            exit_code: window.last_cmd_exit_status,
        });
    }
    Ok(views)
}

async fn get_window_content(
    window: &Terminal,
    backend: &dyn TerminalBackend,
) -> Result<String, PromptError> {
    let id = window.id.clone();
    match backend
        .execute(BackendCmd::Get(GetTextCmd { window_id: id }))
        .await
    {
        Ok(CmdResponse::Text(text)) => Ok(text),
        Ok(CmdResponse::Error(e)) => Err(PromptError::Backend(
            BackendError::Communication(e),
        )),
        Ok(_) => Err(PromptError::Backend(
            BackendError::Communication("unexpected response".into()),
        )),
        Err(e) => Err(PromptError::Backend(e)),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("backend error: {0}")]
    Backend(#[from] BackendError),
}
