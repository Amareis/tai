use async_trait::async_trait;
use kitty_rc::{
    CloseWindowCommand, GetTextCommand, Kitty, KittyBuilder, LaunchCommand, LsCommand,
    SendKeyCommand, SendTextCommand, SetWindowTitleCommand,
};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::Duration;

use crate::types::LaunchOpts;

use super::{BackendError, TerminalBackend, WindowId, WindowInfo};

const KITTY_BINARY: &str = "kitty";
const SOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SOCKET_WAIT_INTERVAL: Duration = Duration::from_millis(100);

/// Kitty terminal backend — управляет окнами через Kitty Remote Control protocol.
///
/// Запускает свой собственный инстанс (`spawn`). При  этом бэкенд владеет child-процессом
/// и убивает его при drop.
///
/// Все `launch`-вызовы гарантированно используют `hold = true` — ядро
/// предотвращает потерю вывода при завершении процесса.
pub struct KittyBackend {
    client: Mutex<Kitty>,
    socket_path: PathBuf,

    #[allow(dead_code)]
    child: Option<Child>,
}

impl KittyBackend {
    /// Запустить новый инстанс Kitty и подключиться к нему.
    ///
    /// Kitty запускается с `allow_remote_control=yes` и socket-ом по заданному пути.
    /// Если `socket_path` — `None`, генерируется путь в temp-директории.
    ///
    /// Метод ждёт появления socket-файла (до `SOCKET_CONNECT_TIMEOUT`).
    pub async fn spawn(
        run_cmd: &[&str],
        socket_path: Option<PathBuf>,
        hidden: bool,
    ) -> Result<Self, BackendError> {
        let socket_path = socket_path.unwrap_or_else(|| {
            std::env::temp_dir().join(format!("tai-kitty-{}.sock", uuid::Uuid::new_v4()))
        });

        let mut cmd = Command::new(KITTY_BINARY);

        cmd.kill_on_drop(true)
            // TODO записывать stderr и что-то с ним делать (см. план)
            .stderr(Stdio::null())
            .arg("-o")
            .arg("allow_remote_control=yes")
            .arg("--listen-on")
            .arg(format!("unix:{}", socket_path.display()));

        if hidden {
            cmd.arg("--start-as=hidden");
        }

        cmd.args(run_cmd);

        tracing::info!("starting KittyBackend, args: {:?}", run_cmd);

        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackendError::LaunchFailed(
                    "kitty not found in PATH. Install kitty terminal emulator.".to_string(),
                )
            } else {
                BackendError::LaunchFailed(format!("failed to spawn kitty: {e}"))
            }
        })?;

        let client = Self::wait_for_socket(&socket_path).await?;

        Ok(Self {
            client: Mutex::new(client),
            child: Some(child),
            socket_path,
        })
    }

    /// Путь к socket, через который идёт связь.
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    async fn wait_for_socket(socket_path: &PathBuf) -> Result<Kitty, BackendError> {
        let deadline = SOCKET_CONNECT_TIMEOUT;
        let start = std::time::Instant::now();

        loop {
            if socket_path.exists() {
                match Self::connect_to_socket(socket_path).await {
                    Ok(client) => return Ok(client),
                    Err(_) if start.elapsed() < deadline => {
                        tokio::time::sleep(SOCKET_WAIT_INTERVAL).await;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }

            if start.elapsed() >= deadline {
                return Err(BackendError::Communication(format!(
                    "kitty socket {} did not appear within {deadline:?}",
                    socket_path.display()
                )));
            }

            tokio::time::sleep(SOCKET_WAIT_INTERVAL).await;
        }
    }

    async fn connect_to_socket(socket_path: &PathBuf) -> Result<Kitty, BackendError> {
        let builder = KittyBuilder::new()
            .socket_path(socket_path)
            .timeout(SOCKET_CONNECT_TIMEOUT);

        builder.connect().await.map_err(|e| {
            BackendError::Communication(format!(
                "failed to connect to kitty socket {}: {e}",
                socket_path.display()
            ))
        })
    }

    fn match_by_id(window: &WindowId) -> String {
        format!("id:{}", window.0)
    }

    fn check_kitty_response(
        response: &kitty_rc::KittyResponse,
        context: &str,
    ) -> Result<(), BackendError> {
        if !response.ok {
            let err = response.error.as_deref().unwrap_or("unknown error");
            return Err(BackendError::Communication(format!("{context}: {err}")));
        }
        Ok(())
    }
}

#[async_trait]
impl TerminalBackend for KittyBackend {
    async fn launch(&self, opts: &LaunchOpts) -> Result<WindowId, BackendError> {
        let msg = LaunchCommand::new()
            .args(opts.args.clone())
            .window_title(&opts.title)
            .hold(true)
            .keep_focus(true)
            .build()
            .map_err(|e| BackendError::LaunchFailed(format!("build launch command: {e}")))?;

        let mut client = self.client.lock().await;
        let response = client
            .execute(&msg)
            .await
            .map_err(|e| BackendError::LaunchFailed(format!("execute launch: {e}")))?;

        Self::check_kitty_response(&response, "launch")?;

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
        let data = format!("text:{text}");

        let msg = SendTextCommand::new(&data)
            .match_spec(&match_spec)
            .build()
            .map_err(|e| BackendError::SendFailed(format!("build send-text: {e}")))?;

        let mut client = self.client.lock().await;
        let response = client
            .execute(&msg)
            .await
            .map_err(|e| BackendError::SendFailed(format!("execute send-text: {e}")))?;

        Self::check_kitty_response(&response, "send-text")?;
        Ok(())
    }

    async fn send_keys(&self, window: &WindowId, keys: &str) -> Result<(), BackendError> {
        let match_spec = Self::match_by_id(window);

        let msg = SendKeyCommand::new(keys)
            .match_spec(&match_spec)
            .build()
            .map_err(|e| BackendError::SendFailed(format!("build send-key: {e}")))?;

        let mut client = self.client.lock().await;
        let response = client
            .execute(&msg)
            .await
            .map_err(|e| BackendError::SendFailed(format!("execute send-key: {e}")))?;

        Self::check_kitty_response(&response, "send-key")?;
        Ok(())
    }

    async fn get_text(&self, window: &WindowId) -> Result<String, BackendError> {
        let match_spec = Self::match_by_id(window);

        let msg = GetTextCommand::new()
            .match_spec(&match_spec)
            .extent("all")
            .ansi(false)
            .build()
            .map_err(|e| BackendError::Communication(format!("build get-text: {e}")))?;

        let mut client = self.client.lock().await;
        let response = client
            .execute(&msg)
            .await
            .map_err(|e| BackendError::Communication(format!("execute get-text: {e}")))?;

        Self::check_kitty_response(&response, "get-text")?;

        let text = response
            .data
            .as_ref()
            .and_then(|d| {
                if let Some(s) = d.as_str() {
                    return Some(s.to_string());
                }
                if let Some(obj) = d.as_object()
                    && let Some(text_val) = obj.get("text")
                {
                    return text_val.as_str().map(std::string::ToString::to_string);
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
            .map_err(|e| BackendError::Communication(format!("build close-window: {e}")))?;

        let mut client = self.client.lock().await;
        let response = client
            .execute(&msg)
            .await
            .map_err(|e| BackendError::Communication(format!("execute close-window: {e}")))?;

        Self::check_kitty_response(&response, "close-window")?;
        Ok(())
    }

    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
        let msg = LsCommand::new()
            .build()
            .map_err(|e| BackendError::Communication(format!("build ls: {e}")))?;

        let mut client = self.client.lock().await;
        let response = client
            .execute(&msg)
            .await
            .map_err(|e| BackendError::Communication(format!("execute ls: {e}")))?;

        Self::check_kitty_response(&response, "ls")?;

        let os_instances = LsCommand::parse_response(&response)
            .map_err(|e| BackendError::Communication(format!("parse ls response: {e}")))?;

        let mut result = Vec::new();
        for os_instance in &os_instances {
            for tab in &os_instance.tabs {
                for win in &tab.windows {
                    let id = win
                        .id
                        .map_or_else(|| WindowId(String::new()), |id| WindowId(id.to_string()));
                    let title = win.title.clone().unwrap_or_default();
                    #[allow(clippy::cast_possible_truncation)]
                    let pid = win.pid.map_or(0, |p| p as u32);
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
            .map_err(|e| BackendError::Communication(format!("build set-window-title: {e}")))?;

        let mut client = self.client.lock().await;
        let response = client
            .execute(&msg)
            .await
            .map_err(|e| BackendError::Communication(format!("execute set-window-title: {e}")))?;

        Self::check_kitty_response(&response, "set-window-title")?;
        Ok(())
    }
}
