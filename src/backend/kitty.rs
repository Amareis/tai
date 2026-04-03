use async_trait::async_trait;
use kitty_rc::{
    CloseWindowCommand, GetTextCommand, Kitty, KittyBuilder, LaunchCommand, LsCommand,
    SendKeyCommand, SendTextCommand, SetWindowTitleCommand,
};
use std::path::PathBuf;
use tokio::sync::Mutex;

use crate::types::LaunchOpts;

use super::{BackendError, TerminalBackend, WindowId, WindowInfo};

/// Kitty terminal backend — управляет окнами через Kitty Remote Control protocol.
///
/// Подключается к Kitty через unix socket (`kitty-*.sock`). Каждая операция
/// захватывает `Mutex` чтобы гарантировать эксклюзивный доступ к `Kitty` client,
/// т.к. протокол Kitty не поддерживает concurrent requests на одном connection.
///
/// Все `launch`-вызовы гарантированно используют `hold = true` — ядро
/// предотвращает потерю вывода при завершении процесса.
pub struct KittyBackend {
    client: Mutex<Kitty>,
}

impl KittyBackend {
    /// Подключиться к Kitty по пути к socket.
    ///
    /// Socket обычно находится в `$XDG_RUNTIME_DIR/kitty-<pid>.sock`
    /// или может быть задан через `--listen-on` при запуске Kitty.
    pub async fn connect(socket_path: PathBuf) -> Result<Self, BackendError> {
        let builder = KittyBuilder::new()
            .socket_path(&socket_path)
            .timeout(std::time::Duration::from_secs(10));

        let client = builder.connect().await.map_err(|e| {
            BackendError::Communication(format!("failed to connect to kitty socket {:?}: {}", socket_path, e))
        })?;

        Ok(Self {
            client: Mutex::new(client),
        })
    }

    /// Подключиться к Kitty по PID процесса.
    ///
    /// Socket path вычисляется как `$XDG_RUNTIME_DIR/kitty-<pid>.sock`.
    pub async fn from_pid(pid: u32) -> Result<Self, BackendError> {
        let builder = KittyBuilder::new()
            .with_pid(pid)
            .timeout(std::time::Duration::from_secs(10));

        let client = builder.connect().await.map_err(|e| {
            BackendError::Communication(format!("failed to connect to kitty pid {}: {}", pid, e))
        })?;

        Ok(Self {
            client: Mutex::new(client),
        })
    }

    fn match_by_id(window: &WindowId) -> String {
        format!("id:{}", window.0)
    }
}

#[async_trait]
impl TerminalBackend for KittyBackend {
    async fn launch(&self, opts: &LaunchOpts) -> Result<WindowId, BackendError> {
        let mut cmd_builder = LaunchCommand::new()
            .args(&opts.command)
            .window_title(&opts.title)
            .hold(true)
            .keep_focus(true);

        if let Some(shell) = &opts.shell {
            cmd_builder = cmd_builder.args(shell);
        }

        let msg = cmd_builder
            .build()
            .map_err(|e| BackendError::LaunchFailed(format!("build launch command: {}", e)))?;

        let mut client = self.client.lock().await;
        let response = client.execute(&msg).await.map_err(|e| {
            BackendError::LaunchFailed(format!("execute launch: {}", e))
        })?;

        if !response.ok {
            let err = response
                .error
                .as_deref()
                .unwrap_or("unknown error");
            return Err(BackendError::LaunchFailed(err.to_string()));
        }

        let window_id = response
            .data
            .as_ref()
            .and_then(|d| {
                if let Some(s) = d.as_str() {
                    return Some(s.to_string());
                }
                if let Some(n) = d.as_u64() {
                    return Some(n.to_string());
                }
                None
            })
            .ok_or_else(|| {
                BackendError::LaunchFailed(
                    "kitty launch returned ok but no window id in response".to_string(),
                )
            })?;

        Ok(WindowId(window_id))
    }

    async fn send_text(&self, window: &WindowId, text: &str) -> Result<(), BackendError> {
        let match_spec = Self::match_by_id(window);
        let data = format!("text:{}", text);

        let msg = SendTextCommand::new(&data)
            .match_spec(&match_spec)
            .build()
            .map_err(|e| BackendError::SendFailed(format!("build send-text: {}", e)))?;

        let mut client = self.client.lock().await;
        let response = client.execute(&msg).await.map_err(|e| {
            BackendError::SendFailed(format!("execute send-text: {}", e))
        })?;

        if !response.ok {
            let err = response.error.as_deref().unwrap_or("unknown error");
            return Err(BackendError::SendFailed(err.to_string()));
        }

        Ok(())
    }

    async fn send_keys(&self, window: &WindowId, keys: &str) -> Result<(), BackendError> {
        let match_spec = Self::match_by_id(window);

        let msg = SendKeyCommand::new(keys)
            .match_spec(&match_spec)
            .build()
            .map_err(|e| BackendError::SendFailed(format!("build send-key: {}", e)))?;

        let mut client = self.client.lock().await;
        let response = client.execute(&msg).await.map_err(|e| {
            BackendError::SendFailed(format!("execute send-key: {}", e))
        })?;

        if !response.ok {
            let err = response.error.as_deref().unwrap_or("unknown error");
            return Err(BackendError::SendFailed(err.to_string()));
        }

        Ok(())
    }

    async fn get_text(&self, window: &WindowId) -> Result<String, BackendError> {
        let match_spec = Self::match_by_id(window);

        let msg = GetTextCommand::new()
            .match_spec(&match_spec)
            .extent("all")
            .ansi(false)
            .build()
            .map_err(|e| BackendError::Communication(format!("build get-text: {}", e)))?;

        let mut client = self.client.lock().await;
        let response = client.execute(&msg).await.map_err(|e| {
            BackendError::Communication(format!("execute get-text: {}", e))
        })?;

        if !response.ok {
            let err = response.error.as_deref().unwrap_or("unknown error");
            return Err(BackendError::Communication(err.to_string()));
        }

        let text = response
            .data
            .as_ref()
            .and_then(|d| {
                if let Some(s) = d.as_str() {
                    return Some(s.to_string());
                }
                if let Some(obj) = d.as_object() {
                    if let Some(text_val) = obj.get("text") {
                        return text_val.as_str().map(|s| s.to_string());
                    }
                }
                None
            })
            .unwrap_or_default();

        Ok(text)
    }

    async fn close(&self, window: &WindowId) -> Result<(), BackendError> {
        let match_spec = Self::match_by_id(window);

        let msg = CloseWindowCommand::new()
            .match_spec(&match_spec)
            .build()
            .map_err(|e| BackendError::Communication(format!("build close-window: {}", e)))?;

        let mut client = self.client.lock().await;
        let response = client.execute(&msg).await.map_err(|e| {
            BackendError::Communication(format!("execute close-window: {}", e))
        })?;

        if !response.ok {
            let err = response.error.as_deref().unwrap_or("unknown error");
            return Err(BackendError::Communication(err.to_string()));
        }

        Ok(())
    }

    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
        let msg = LsCommand::new()
            .build()
            .map_err(|e| BackendError::Communication(format!("build ls: {}", e)))?;

        let mut client = self.client.lock().await;
        let response = client.execute(&msg).await.map_err(|e| {
            BackendError::Communication(format!("execute ls: {}", e))
        })?;

        if !response.ok {
            let err = response.error.as_deref().unwrap_or("unknown error");
            return Err(BackendError::Communication(err.to_string()));
        }

        let os_instances = LsCommand::parse_response(&response)
            .map_err(|e| BackendError::Communication(format!("parse ls response: {}", e)))?;

        let mut result = Vec::new();
        for os_instance in &os_instances {
            for tab in &os_instance.tabs {
                for win in &tab.windows {
                    let id = win
                        .id
                        .map(|id| WindowId(id.to_string()))
                        .unwrap_or_else(|| WindowId(String::new()));
                    let title = win.title.clone().unwrap_or_default();
                    let pid = win.pid.map(|p| p as u32).unwrap_or(0);
                    let is_at_prompt = win.at_prompt.unwrap_or(false);

                    result.push(WindowInfo {
                        id,
                        title,
                        pid,
                        is_at_prompt,
                    });
                }
            }
        }

        Ok(result)
    }

    async fn set_title(&self, window: &WindowId, title: &str) -> Result<(), BackendError> {
        let match_spec = Self::match_by_id(window);

        let msg = SetWindowTitleCommand::new(title)
            .match_spec(&match_spec)
            .build()
            .map_err(|e| BackendError::Communication(format!("build set-window-title: {}", e)))?;

        let mut client = self.client.lock().await;
        let response = client.execute(&msg).await.map_err(|e| {
            BackendError::Communication(format!("execute set-window-title: {}", e))
        })?;

        if !response.ok {
            let err = response.error.as_deref().unwrap_or("unknown error");
            return Err(BackendError::Communication(err.to_string()));
        }

        Ok(())
    }
}
