use crate::backend::Terminal;
use serde::{Deserialize, Serialize};

/// Триггер нового тика (цикла ядра).
///
/// Тик — один проход assemble → invoke → parse → execute → wait.
/// Триггер определяет, что запускает новый тик.
/// TODO: Phase 6 — использовать в kernel/mod.rs
#[derive(Debug, Clone, PartialEq)]
pub enum TickTrigger {
    /// Терминальное окно завершилось (`at_prompt` == true)
    WindowsExited(Vec<Terminal>),
    /// Пользователь написал сообщение в TUI chat
    UserMessage,
    /// Прошёл idle-timeout без событий
    IdleTimeout,
}

/// Режим code block в ответе модели.
///
/// Без суффикса (по умолчанию) — text. С суффиксом — указанное действие.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockMode {
    /// Текст в stdin окна (auto-launch если не существует)
    Text,
    /// Закрыть окно
    Close,
}

impl std::str::FromStr for BlockMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "close" => Ok(Self::Close),
            "text" => Ok(Self::Text),
            _ => Err(format!("unknown block mode: {s}")),
        }
    }
}

impl std::fmt::Display for BlockMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockMode::Text => f.write_str("text"),
            BlockMode::Close => f.write_str("close"),
        }
    }
}

/// Сегмент ответа модели после парсинга.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParsedSegment {
    /// Code block: ` ```title[:mode]\ncontent\n``` `
    ///
    /// По умолчанию mode=text (send to stdin, auto-launch).
    /// mode=close — закрыть окно.
    Block {
        window: String,
        mode: BlockMode,
        content: String,
    },
    /// Текст вне блоков — prose, сохраняется для контекста
    Prose(String),
}
