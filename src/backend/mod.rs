pub mod kitty;
pub mod watch;

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use std::fmt;

/// Уникальный идентификатор окна в backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowId(pub String);

impl WindowId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for WindowId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(WindowId(s.to_string()))
    }
}

/// Информация об окне, возвращаемая backend при `list_windows`.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    pub pid: u32,
    pub is_at_prompt: bool,
}

/// Команда для запуска нового терминального окна.
#[derive(Debug, Clone, PartialEq, Parser)]
pub struct LaunchCmd {
    /// Заголовок окна
    #[arg(short, long)]
    pub title: Option<String>,
    /// Команда для запуска (argv)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

/// Команда отправки текста в stdin окна.
#[derive(Debug, Clone, PartialEq, Parser)]
pub struct SendTextCmd {
    /// ID окна
    pub window: WindowId,
    /// Текст для отправки
    #[arg(trailing_var_arg = true)]
    pub text: Vec<String>,
}

/// Команда отправки клавиш в окно.
#[derive(Debug, Clone, PartialEq, Parser)]
pub struct SendKeysCmd {
    /// ID окна
    pub window: WindowId,
    /// Клавиши для отправки
    #[arg(trailing_var_arg = true)]
    pub keys: Vec<String>,
}

/// Команда получения содержимого окна.
#[derive(Debug, Clone, PartialEq, Parser)]
pub struct GetTextCmd {
    /// ID окна
    pub window: WindowId,
}

/// Команда закрытия окна.
#[derive(Debug, Clone, PartialEq, Parser)]
pub struct CloseCmd {
    /// ID окна
    pub window: WindowId,
}

/// Команда установки заголовка окна.
#[derive(Debug, Clone, PartialEq, Parser)]
pub struct SetTitleCmd {
    /// ID окна
    pub window: WindowId,
    /// Новый заголовок
    #[arg(trailing_var_arg = true)]
    pub title: Vec<String>,
}

/// Команда для терминального backend.
///
/// Человек (и модель через code blocks) пишет текстовые команды.
/// Clap парсит строку в `BackendCmd` enum.
/// Ошибка парсинга → сразу feedback, даже до backend'а не доходит.
/// Успех → `backend.execute(cmd)`.
#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum BackendCmd {
    /// Запустить новое терминальное окно
    Launch(LaunchCmd),
    /// Отправить текст в stdin окна
    Send(SendTextCmd),
    /// Отправить клавиши в окно
    Keys(SendKeysCmd),
    /// Получить содержимое окна
    Get(GetTextCmd),
    /// Закрыть окно
    Close(CloseCmd),
    /// Получить список окон
    List,
    /// Установить заголовок окна
    Title(SetTitleCmd),
}

/// Ответ на команду backend.
#[derive(Debug, Clone)]
pub enum CmdResponse {
    /// Окно создано
    WindowCreated(WindowId),
    /// Текст содержимого окна
    Text(String),
    /// Список окон
    Windows(Vec<WindowInfo>),
    /// Команда выполнена успешно
    Ok,
    /// Ошибка выполнения
    Error(String),
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
pub trait TerminalBackend: Send + Sync {
    /// Выполнить команду backend.
    ///
    /// Единая точка входа для всех операций с терминалом.
    /// Pipeline: строка → clap → `BackendCmd` → `execute()` → `CmdResponse`
    async fn execute(&self, cmd: BackendCmd) -> Result<CmdResponse, BackendError>;
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
