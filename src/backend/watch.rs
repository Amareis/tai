use std::collections::HashMap;

use chrono::Utc;
use tracing::info;

use crate::backend::{BackendCmd, BackendError, CloseCmd, CmdResponse, GetTextCmd, TerminalBackend, WindowId};
use crate::types::{Session, Window, WindowState};

#[derive(Debug, Clone)]
pub enum WatchEvent {
    WindowAdded { window_id: String },
    WindowExited { window_id: String, exit_code: i32 },
}

struct TrackedWindow {
    title: String,
    added_to_session: bool,
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

    pub fn track(&mut self, backend_id: WindowId, title: String) {
        info!("watcher: tracking {backend_id} ({title})");
        self.tracked.insert(backend_id, TrackedWindow {
            title,
            added_to_session: false,
        });
    }

    pub async fn poll(
        &mut self,
        back: &dyn TerminalBackend,
        session: &mut Session,
    ) -> Result<Vec<WatchEvent>, BackendError> {
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

        let window_map: HashMap<&WindowId, &crate::backend::WindowInfo> = all_windows
            .iter()
            .map(|w| (&w.id, w))
            .collect();

        let mut to_add = Vec::new();
        let mut to_freeze = Vec::new();

        for (backend_id, tracked) in &self.tracked {
            if let Some(info) = window_map.get(backend_id) {
                if !tracked.added_to_session {
                    to_add.push((backend_id.clone(), info.pid, tracked.title.clone()));
                } else if info.is_at_prompt {
                    to_freeze.push((backend_id.clone(), info.last_cmd_exit_status));
                }
            }
        }

        let mut events = Vec::new();

        for (backend_id, pid, title) in to_add {
            let window = Window::new_active(backend_id.0.clone(), pid, title);
            let window_id = window.id.clone();
            session.windows.push(window);

            if let Some(t) = self.tracked.get_mut(&backend_id) {
                t.added_to_session = true;
            }

            info!("watcher: added window {window_id} (backend {backend_id})");
            events.push(WatchEvent::WindowAdded { window_id });
        }

        for (backend_id, exit_code) in to_freeze {
            let content = back
                .execute(BackendCmd::Get(GetTextCmd {
                    window: backend_id.clone(),
                }))
                .await
                .map(|r| match r {
                    CmdResponse::Text(t) => t,
                    _ => String::new(),
                })
                .unwrap_or_default();

            let exit_code = exit_code.unwrap_or(-1);

            if let Some(w) = session.windows.iter_mut().find(|w| {
                matches!(&w.state, WindowState::Active { backend_id: bid, .. } if bid == &backend_id.0)
            }) {
                w.state = WindowState::Frozen {
                    content,
                    exit_code,
                    captured_at: Utc::now(),
                };
                let wid = w.id.clone();
                info!("watcher: froze window {wid} (exit_code={exit_code})");
                events.push(WatchEvent::WindowExited { window_id: wid, exit_code });
            }

            let _ = back
                .execute(BackendCmd::Close(CloseCmd {
                    window: backend_id.clone(),
                }))
                .await;

            self.tracked.remove(&backend_id);
        }

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendCmd, BackendError, CmdResponse, TerminalBackend, WindowId, WindowInfo};
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

        fn add_window(&self, id: &str, at_prompt: bool, text: &str) {
            let mut windows = self.windows.lock().unwrap();
            windows.push(MockWindow {
                id: WindowId(id.to_string()),
                title: id.to_string(),
                at_prompt,
                last_cmd_exit_status: None,
                text: text.to_string(),
            });
        }

        fn add_window_with_exit(&self, id: &str, at_prompt: bool, text: &str, exit_code: i32) {
            let mut windows = self.windows.lock().unwrap();
            windows.push(MockWindow {
                id: WindowId(id.to_string()),
                title: id.to_string(),
                at_prompt,
                last_cmd_exit_status: Some(exit_code),
                text: text.to_string(),
            });
        }

        fn set_at_prompt(&self, id: &str, at_prompt: bool) {
            let mut windows = self.windows.lock().unwrap();
            if let Some(w) = windows.iter_mut().find(|w| w.id.0 == id) {
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
                        .find(|w| w.id == cmd.window)
                        .ok_or_else(|| BackendError::WindowNotFound(cmd.window.to_string()))?;
                    Ok(CmdResponse::Text(w.text.clone()))
                }
                BackendCmd::Close(cmd) => {
                    let mut windows = self.windows.lock().unwrap();
                    windows.retain(|w| w.id != cmd.window);
                    Ok(CmdResponse::Ok)
                }
                BackendCmd::List => {
                    let windows = self.windows.lock().unwrap();
                    Ok(CmdResponse::Windows(
                        windows
                            .iter()
                            .map(|w| WindowInfo {
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
    }

    fn empty_session() -> Session {
        Session::new("test".to_string(), std::env::temp_dir().join("tai-test-mind.md"))
    }

    #[tokio::test]
    async fn test_poll_empty_tracked() {
        let backend = MockBackend::new();
        let mut watcher = Watcher::new();
        let mut session = empty_session();

        let events = watcher.poll(&backend, &mut session).await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_poll_adds_window_to_session() {
        let backend = MockBackend::new();
        backend.add_window("w1", false, "running...");

        let mut watcher = Watcher::new();
        let mut session = empty_session();
        watcher.track(WindowId("w1".to_string()), "hello".to_string());

        let events = watcher.poll(&backend, &mut session).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], WatchEvent::WindowAdded { window_id } if !window_id.is_empty()));

        assert_eq!(session.windows.len(), 1);
        assert!(session.windows[0].state.is_active());
    }

    #[tokio::test]
    async fn test_poll_freezes_on_exit() {
        let backend = MockBackend::new();
        backend.add_window_with_exit("w1", true, "hello world", 0);

        let mut watcher = Watcher::new();
        let mut session = empty_session();
        watcher.track(WindowId("w1".to_string()), "hello".to_string());

        // first poll: add to session
        let events = watcher.poll(&backend, &mut session).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(session.windows[0].state.is_active());

        backend.set_at_prompt("w1", true);

        // second poll: freeze
        let events = watcher.poll(&backend, &mut session).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], WatchEvent::WindowExited { exit_code: 0, .. }));
        assert!(session.windows[0].state.is_frozen());
    }

    #[tokio::test]
    async fn test_poll_no_at_prompt_stays_active() {
        let backend = MockBackend::new();
        backend.add_window("w1", false, "running...");

        let mut watcher = Watcher::new();
        let mut session = empty_session();
        watcher.track(WindowId("w1".to_string()), "hello".to_string());

        // first poll: add
        let events = watcher.poll(&backend, &mut session).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(session.windows[0].state.is_active());

        // second poll: still running (not at prompt)
        let events = watcher.poll(&backend, &mut session).await.unwrap();
        assert!(events.is_empty());
        assert!(session.windows[0].state.is_active());
    }
}