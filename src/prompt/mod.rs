use crate::backend::{BackendCmd, CmdResponse, GetTextCmd, TerminalBackend};
use crate::types::{Session, Window, WindowState};

const SYSTEM_PROMPT: &str = "You are TAI, a terminal agent. You control terminal windows.

Format your response with code blocks:
- ```<window_id>:text\ncommand\n``` — send text to window stdin
- ```<window_id>:keys\nkey1 key2\n``` — send keypresses
- ```tai:cmd\nlaunch --title name -- command\n``` — TAI commands

Text outside blocks goes to chat.

When a process finishes (exit code shown), analyze the result and decide next steps.";

pub async fn build(
    session: &Session,
    backend: &dyn TerminalBackend,
    previous_response: Option<&str>,
) -> Result<String, PromptError> {
    let mut parts = Vec::new();

    parts.push(SYSTEM_PROMPT.to_string());
    parts.push(String::new());
    parts.push(build_dashboard(session));

    for window in session.focused_windows() {
        let content = get_window_content(window, backend).await?;
        parts.push(format_window(window, &content));
    }

    if let Some(prev) = previous_response {
        parts.push(String::new());
        parts.push("## Your previous response".to_string());
        parts.push(prev.to_string());
    }

    parts.push(String::new());
    parts.push(build_status_bar(session));

    Ok(parts.join("\n"))
}

fn build_dashboard(session: &Session) -> String {
    let mut lines = vec!["## Dashboard".to_string()];

    if session.windows.is_empty() {
        lines.push("No windows.".to_string());
    } else {
        for w in &session.windows {
            let status = match &w.state {
                WindowState::Active { backend_id, title, .. } => {
                    format!("[active] {title} ({backend_id})")
                }
                WindowState::Frozen { exit_code, .. } => {
                    format!("[frozen] exit code: {exit_code}")
                }
                WindowState::Archived { .. } => "[archived]".to_string(),
            };
            let focus = if w.focused { " *" } else { "" };
            lines.push(format!("- [{}] {}{}", w.id, status, focus));
        }
    }

    lines.join("\n")
}

async fn get_window_content(
    window: &Window,
    backend: &dyn TerminalBackend,
) -> Result<String, PromptError> {
    match &window.state {
        WindowState::Active { backend_id, .. } => {
            let id = crate::backend::WindowId(backend_id.clone());
            match backend
                .execute(BackendCmd::Get(GetTextCmd {
                    window: id,
                }))
                .await
            {
                Ok(CmdResponse::Text(text)) => Ok(text),
                Ok(CmdResponse::Error(e)) => {
                    Err(PromptError::Backend(crate::backend::BackendError::Communication(e)))
                }
                Ok(_) => {
                    Err(PromptError::Backend(crate::backend::BackendError::Communication(
                        "unexpected response".into(),
                    )))
                }
                Err(e) => Err(PromptError::Backend(e)),
            }
        }
        WindowState::Frozen { content, .. } => Ok(content.clone()),
        WindowState::Archived { .. } => Ok("[archived]".to_string()),
    }
}

fn format_window(window: &Window, content: &str) -> String {
    let title = match &window.state {
        WindowState::Active { title, .. } => title.clone(),
        WindowState::Frozen { .. } => "frozen".to_string(),
        WindowState::Archived { .. } => "archived".to_string(),
    };

    let exit_info = window.state.exit_code().map_or(String::new(), |c| {
        format!("\n**Exit code: {c}**")
    });

    format!(
        "## Window [{}] {} (focused)\n{}{}\n```\n{}\n```",
        window.id, title, exit_info, if exit_info.is_empty() { "" } else { "\n" }, content
    )
}

fn build_status_bar(session: &Session) -> String {
    let active = session.active_windows().count();
    let frozen = session
        .windows
        .iter()
        .filter(|w| w.state.is_frozen())
        .count();
    let focused = session.focused_windows().count();

    format!(
        "[STATUS] Active: {active} | Frozen: {frozen} | Focused: {focused}"
    )
}

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("backend error: {0}")]
    Backend(#[from] crate::backend::BackendError),
}
