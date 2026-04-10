use crate::backend::Terminal;

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
/// Определяет как именно содержимое блока будет отправлено в окно.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockMode {
    /// Текст в stdin окна (`send-text`)
    Text,
    /// Клавиши в окно (`send-key`)
    Keys,
    /// Команда ядра TAI (`tai:cmd`)
    Cmd,
}

impl std::str::FromStr for BlockMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(Self::Text),
            "keys" => Ok(Self::Keys),
            "cmd" | "tai:cmd" => Ok(Self::Cmd),
            _ => Err(format!("unknown block mode: {s}")),
        }
    }
}

impl std::fmt::Display for BlockMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockMode::Text => f.write_str("text"),
            BlockMode::Keys => f.write_str("keys"),
            BlockMode::Cmd => f.write_str("cmd"),
        }
    }
}

/// Сегмент ответа модели после парсинга.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedSegment {
    Reasoning(String),
    /// Code block: ` ```window:mode\ncontent\n``` `
    Block {
        window: String,
        mode: BlockMode,
        content: String,
    },
    /// Текст вне блоков — prose в чат
    Prose(String),
}
