use std::collections::HashMap;

use tracing::info;

use crate::backend::{BackendCmd, BackendError, CmdResponse, Terminal, TerminalBackend, WindowId};

struct TrackedWindow {
    #[allow(dead_code)]
    id: WindowId,
    was_active: bool,
}

pub struct Watcher {
    tracked: HashMap<WindowId, TrackedWindow>,
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Watcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracked: HashMap::new(),
        }
    }

    pub fn track(&mut self, id: WindowId, is_active: bool) {
        info!("watcher: tracking {id}");
        self.tracked.insert(
            id.clone(),
            TrackedWindow {
                id,
                was_active: is_active,
            },
        );
    }

    pub async fn poll_exited(
        &mut self,
        back: &dyn TerminalBackend,
    ) -> Result<Vec<Terminal>, BackendError> {
        if self.tracked.is_empty() {
            return Ok(Vec::new());
        }

        let all_windows = match back.execute(BackendCmd::List).await? {
            CmdResponse::Windows(w) => w,
            CmdResponse::Error(e) => return Err(BackendError::Communication(e)),
            _ => {
                return Err(BackendError::Communication(
                    "unexpected response for List".into(),
                ));
            }
        };

        let window_map: HashMap<&WindowId, &Terminal> =
            all_windows.iter().map(|w| (&w.id, w)).collect();

        let mut exited = Vec::<Terminal>::new();

        for (backend_id, tracked) in &mut self.tracked {
            if let Some(info) = window_map.get(backend_id)
                && info.is_at_prompt
                && tracked.was_active
            {
                exited.push((*info).clone());
                tracked.was_active = false;
            }
        }

        //TODO close exited windows?
        // let _ = back
        //     .execute(BackendCmd::Close(CloseCmd {
        //         window: backend_id.clone(),
        //     }))
        //     .await;
        //
        // self.tracked.remove(&backend_id);

        Ok(exited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendCmd, BackendError, CmdResponse, LaunchCmd, Terminal, TerminalBackend, WindowId};
    use assert_matches::assert_matches;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct MockBackend {
        windows: Arc<std::sync::Mutex<Vec<MockWindow>>>,
        get_text_calls: Arc<AtomicUsize>,
    }

    struct MockWindow {
        id: WindowId,
        title: String,
        at_prompt: bool,
        last_cmd_exit_status: Option<i32>,
        text: String,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                windows: Arc::new(std::sync::Mutex::new(Vec::new())),
                get_text_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn add_window(&self, id: WindowId, at_prompt: bool, text: &str) {
            let mut windows = self.windows.lock().unwrap();
            windows.push(MockWindow {
                title: id.to_string(),
                id,
                at_prompt,
                last_cmd_exit_status: None,
                text: text.to_string(),
            });
        }

        fn add_window_with_exit(&self, id: WindowId, at_prompt: bool, text: &str, exit_code: i32) {
            let mut windows = self.windows.lock().unwrap();
            windows.push(MockWindow {
                title: id.to_string(),
                id,
                at_prompt,
                last_cmd_exit_status: Some(exit_code),
                text: text.to_string(),
            });
        }

        fn set_at_prompt(&self, id: &WindowId, at_prompt: bool) {
            let mut windows = self.windows.lock().unwrap();
            if let Some(w) = windows.iter_mut().find(|w| w.id == *id) {
                w.at_prompt = at_prompt;
            }
        }
    }

    #[async_trait]
    impl TerminalBackend for MockBackend {
        async fn execute(&self, cmd: BackendCmd) -> Result<CmdResponse, BackendError> {
            match cmd {
                BackendCmd::Launch(_) => {
                    Ok(CmdResponse::WindowCreated(WindowId("mock-1".to_string())))
                }
                BackendCmd::Send(_) | BackendCmd::Keys(_) | BackendCmd::Title(_) => {
                    Ok(CmdResponse::Ok)
                }
                BackendCmd::Get(cmd) => {
                    self.get_text_calls.fetch_add(1, Ordering::SeqCst);
                    let windows = self.windows.lock().unwrap();
                    let w = windows
                        .iter()
                        .find(|w| w.id == cmd.window_id)
                        .ok_or_else(|| BackendError::WindowNotFound(cmd.window_id.to_string()))?;
                    Ok(CmdResponse::Text(w.text.clone()))
                }
                BackendCmd::Close(cmd) => {
                    let mut windows = self.windows.lock().unwrap();
                    windows.retain(|w| w.id != cmd.window_id);
                    Ok(CmdResponse::Ok)
                }
                BackendCmd::List => {
                    let windows = self.windows.lock().unwrap();
                    Ok(CmdResponse::Windows(
                        windows
                            .iter()
                            .map(|w| Terminal {
                                id: w.id.clone(),
                                title: w.title.clone(),
                                pid: 1,
                                is_at_prompt: w.at_prompt,
                                last_cmd_exit_status: w.last_cmd_exit_status,
                            })
                            .collect(),
                    ))
                }
            }
        }

        async fn cmd_launch(&self, _cmd: &LaunchCmd) -> Result<WindowId, BackendError> {
            Ok(WindowId("mock-1".to_string()))
        }
    }

    #[tokio::test]
    async fn test_poll_empty_tracked() {
        let backend = MockBackend::new();
        let mut watcher = Watcher::new();

        let events = watcher.poll_exited(&backend).await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_poll_adds_window_to_session() {
        let backend = MockBackend::new();
        let wid = WindowId::new("w1");
        backend.add_window(wid.clone(), false, "running...");

        let mut watcher = Watcher::new();
        watcher.track(wid.clone(), true);

        let exited = watcher.poll_exited(&backend).await.unwrap();
        assert_eq!(exited.len(), 0);
    }

    #[tokio::test]
    async fn test_poll_freezes_on_exit() {
        let backend = MockBackend::new();
        let wid = WindowId::new("w1");
        backend.add_window_with_exit(wid.clone(), false, "hello world", 0);

        let mut watcher = Watcher::new();
        watcher.track(wid.clone(), true);

        // first poll: add to session
        let exited = watcher.poll_exited(&backend).await.unwrap();
        assert_eq!(exited.len(), 0);

        backend.set_at_prompt(&wid, true);

        // second poll: freeze
        let exited = watcher.poll_exited(&backend).await.unwrap();
        assert_eq!(exited.len(), 1);
        assert_eq!(exited[0].id, wid);
        assert_matches!(
            &exited[0],
            Terminal {
                last_cmd_exit_status: Some(0),
                ..
            }
        );
    }

    #[tokio::test]
    async fn test_poll_no_at_prompt_stays_active() {
        let backend = MockBackend::new();
        let wid = WindowId::new("w1");
        backend.add_window(wid, false, "running...");

        let mut watcher = Watcher::new();
        watcher.track(WindowId("w1".to_string()), true);

        // first poll: add
        let exited = watcher.poll_exited(&backend).await.unwrap();
        assert!(exited.is_empty());

        // second poll: still running (not at prompt)
        let exited = watcher.poll_exited(&backend).await.unwrap();
        assert!(exited.is_empty());
    }
}
