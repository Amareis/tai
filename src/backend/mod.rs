use crate::types::LaunchOpts;
use async_trait::async_trait;
use std::fmt;

/// Уникальный идентификатор окна в backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowId(pub String);

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Информация об окне, возвращаемая backend при list_windows.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    pub pid: u32,
    pub is_at_prompt: bool,
}

/// Trait для работы с терминальным backend.
///
/// Модель не вызывает терминальные команды напрямую — она пишет команды ядра TAI
/// через `tai:cmd` code blocks. Ядро парсит, валидирует и мапит на конкретный backend.
///
/// Зачем абстракция:
/// - Модель может забыть `--hold` → окно пропадёт, вывод потерян
/// - Модель не знает путь к сокету → команда упадёт
/// - Хотим поменять Kitty на tmux → не переписываем промпты
/// - Хотим remote → модель не должна знать про SSH
///
/// Реализации: `KittyBackend` (kitty-rc), `TmuxBackend` (future), `RemoteBackend` (future).
#[async_trait]
pub trait TerminalBackend {
    /// Запустить новое терминальное окно с заданными параметрами.
    /// Ядро гарантирует `--hold` для всех launch-вызовов.
    async fn launch(&self, opts: &LaunchOpts) -> Result<WindowId, BackendError>;

    /// Отправить текст в stdin окна (для bash, REPL и т.д.)
    async fn send_text(&self, window: &WindowId, text: &str) -> Result<(), BackendError>;

    /// Отправить keystrokes в окно (для vim, htop, less и других TUI)
    async fn send_keys(&self, window: &WindowId, keys: &str) -> Result<(), BackendError>;

    /// Получить текущее текстовое содержимое окна
    async fn get_text(&self, window: &WindowId) -> Result<String, BackendError>;

    /// Закрыть терминальное окно
    async fn close(&self, window: &WindowId) -> Result<(), BackendError>;

    /// Получить список всех окон backend'а
    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError>;

    /// Установить заголовок окна
    async fn set_title(&self, window: &WindowId, title: &str) -> Result<(), BackendError>;
}

/// Ошибки терминального backend.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("window not found: {0}")]
    WindowNotFound(String),

    #[error("launch failed: {0}")]
    LaunchFailed(String),

    #[error("send failed: {0}")]
    SendFailed(String),

    #[error("communication error: {0}")]
    Communication(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
