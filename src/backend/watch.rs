use std::collections::HashSet;
use std::time::Duration;

use tokio::time;

use crate::backend::{BackendError, TerminalBackend, WindowId};

/// Событие завершения окна — процесс дошёл до prompt.
#[derive(Debug, Clone)]
pub struct WindowExitedEvent {
    pub window_id: WindowId,
    pub content: String,
}

/// Process watch — периодический опрос backend для обнаружения завершившихся окон.
///
/// Kitty запускает окна с `--hold`, поэтому при завершении процесса окно переходит
/// в состояние "at prompt". ProcessWatch опрашивает `list_windows` каждые `poll_interval`
/// и для окон с `at_prompt == true`:
/// 1. Захватывает содержимое через `get_text`
/// 2. Закрывает окно через `close`
/// 3. Возвращает `WindowExitedEvent` с захваченным выводом
///
/// Тrack-множество содержит `WindowId` окон, за которыми ведётся наблюдение.
pub struct ProcessWatch<B: TerminalBackend> {
    backend: B,
    tracked: HashSet<WindowId>,
    poll_interval: Duration,
}

impl<B: TerminalBackend> ProcessWatch<B> {
    pub fn new(backend: B, poll_interval: Duration) -> Self {
        Self {
            backend,
            tracked: HashSet::new(),
            poll_interval,
        }
    }

    /// Добавить окно в список отслеживаемых.
    pub fn track(&mut self, window_id: WindowId) {
        self.tracked.insert(window_id);
    }

    /// Убрать окно из списка отслеживаемых.
    pub fn untrack(&mut self, window_id: &WindowId) {
        self.tracked.remove(window_id);
    }

    /// Сколько окон отслеживается.
    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }

    /// Один цикл опроса — проверить все отслеживаемые окна.
    ///
    /// Возвращает список завершившихся окон с захваченным выводом.
    /// Завершённые окна автоматически удаляются из track-множества.
    pub async fn poll(&mut self) -> Result<Vec<WindowExitedEvent>, BackendError> {
        if self.tracked.is_empty() {
            return Ok(Vec::new());
        }

        let all_windows = self.backend.list_windows().await?;

        let at_prompt_ids: HashSet<WindowId> = all_windows
            .into_iter()
            .filter(|w| w.is_at_prompt && self.tracked.contains(&w.id))
            .map(|w| w.id)
            .collect();

        if at_prompt_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut events = Vec::new();
        let mut to_remove = Vec::new();

        for window_id in &at_prompt_ids {
            let content = match self.backend.get_text(window_id).await {
                Ok(text) => text,
                Err(e) => {
                    tracing::warn!("get_text failed for window {}: {}", window_id, e);
                    String::new()
                }
            };

            if let Err(e) = self.backend.close(window_id).await {
                tracing::warn!("close failed for window {}: {}", window_id, e);
            }

            events.push(WindowExitedEvent {
                window_id: window_id.clone(),
                content,
            });

            to_remove.push(window_id.clone());
        }

        for id in to_remove {
            self.tracked.remove(&id);
        }

        Ok(events)
    }

    /// Бесконечный цикл опроса. Вызывает `handler` для каждого завершившегося окна.
    ///
    /// Прерывается при ошибке связи с backend (возвращает ошибку).
    pub async fn run<F, Fut>(&mut self, handler: F) -> Result<(), BackendError>
    where
        F: Fn(WindowExitedEvent) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let mut interval = time::interval(self.poll_interval);

        loop {
            interval.tick().await;

            let events = self.poll().await?;

            for event in events {
                handler(event).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendError, WindowInfo};
    use crate::types::LaunchOpts;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Clone)]
    struct MockBackend {
        windows: Arc<std::sync::Mutex<Vec<MockWindow>>>,
        get_text_calls: Arc<AtomicUsize>,
    }

    struct MockWindow {
        id: WindowId,
        title: String,
        at_prompt: bool,
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
        async fn launch(&self, _opts: &LaunchOpts) -> Result<WindowId, BackendError> {
            Ok(WindowId("mock-1".to_string()))
        }
        async fn send_text(
            &self,
            _window: &WindowId,
            _text: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn send_keys(
            &self,
            _window: &WindowId,
            _keys: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn get_text(&self, window: &WindowId) -> Result<String, BackendError> {
            self.get_text_calls.fetch_add(1, Ordering::SeqCst);
            let windows = self.windows.lock().unwrap();
            let w = windows
                .iter()
                .find(|w| w.id == *window)
                .ok_or_else(|| BackendError::WindowNotFound(window.to_string()))?;
            Ok(w.text.clone())
        }
        async fn close(&self, window: &WindowId) -> Result<(), BackendError> {
            let mut windows = self.windows.lock().unwrap();
            windows.retain(|w| w.id != *window);
            Ok(())
        }
        async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
            let windows = self.windows.lock().unwrap();
            Ok(windows
                .iter()
                .map(|w| WindowInfo {
                    id: w.id.clone(),
                    title: w.title.clone(),
                    pid: 1,
                    is_at_prompt: w.at_prompt,
                })
                .collect())
        }
        async fn set_title(
            &self,
            _window: &WindowId,
            _title: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_poll_empty_tracked() {
        let backend = MockBackend::new();
        let mut watch = ProcessWatch::new(backend, Duration::from_millis(50));

        let events = watch.poll().await.unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_poll_no_at_prompt() {
        let backend = MockBackend::new();
        backend.add_window("w1", false, "running...");

        let mut watch = ProcessWatch::new(backend, Duration::from_millis(50));
        watch.track(WindowId("w1".to_string()));

        let events = watch.poll().await.unwrap();
        assert!(events.is_empty());
        assert_eq!(watch.tracked_count(), 1);
    }

    #[tokio::test]
    async fn test_poll_detects_at_prompt() {
        let backend = MockBackend::new();
        backend.add_window("w1", true, "hello world\n$ ");

        let mut watch = ProcessWatch::new(backend, Duration::from_millis(50));
        watch.track(WindowId("w1".to_string()));

        let events = watch.poll().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].window_id.0, "w1");
        assert!(events[0].content.contains("hello world"));
        assert_eq!(watch.tracked_count(), 0);
    }

    #[tokio::test]
    async fn test_poll_untrack_on_exit() {
        let backend = MockBackend::new();
        backend.add_window("w1", true, "done");
        backend.add_window("w2", false, "still running");

        let mut watch = ProcessWatch::new(backend, Duration::from_millis(50));
        watch.track(WindowId("w1".to_string()));
        watch.track(WindowId("w2".to_string()));

        let events = watch.poll().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(watch.tracked_count(), 1);
        assert!(watch.tracked.contains(&WindowId("w2".to_string())));
    }

    #[tokio::test]
    async fn test_poll_late_at_prompt() {
        let backend = MockBackend::new();
        backend.add_window("w1", false, "running...");

        let mut watch = ProcessWatch::new(backend.clone(), Duration::from_millis(50));
        watch.track(WindowId("w1".to_string()));

        let events = watch.poll().await.unwrap();
        assert!(events.is_empty());

        backend.set_at_prompt("w1", true);

        let events = watch.poll().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content, "running...");
    }
}
