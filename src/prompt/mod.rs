use crate::agent::AgentResponse;
use crate::backend::{
    BackendCmd, BackendError, CmdResponse, GetTextCmd, Terminal, TerminalBackend, WindowId,
};

const SYSTEM_PROMPT: &str = r"You are TAI, a terminal agent. You control terminal windows.

Format your response with code blocks:
- ```<window_title>\ncommand\n``` — send text to window stdin (auto-creates window if not exists)
- ```<window_title>:close\n``` — close window by title

Use heredoc for writing files:
```build
cat > config.yaml << 'EOF'
server:
  port: 8080
EOF
```

Prose text outside blocks is for context only, not shown to user.

When a process finishes (exit code shown), analyze the result and decide next steps.
Your action history is preserved for one step only. Analyze carefully.

ALL USER MESSAGES ARE AUTOMATIC. Look at the task terminal for instructions.
";

#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub system: String,
    pub dashboard: Vec<Terminal>,
    pub focused_windows: Vec<WindowView>,
    pub previous_response: Option<AgentResponse>,
}

#[derive(Debug, Clone)]
pub struct WindowView {
    pub id: WindowId,
    pub title: String,
    pub content: String,
    pub exit_code: Option<i32>,
}

impl Prompt {
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
