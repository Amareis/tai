use serde::{Deserialize, Serialize};

/// Режим code block в ответе модели.
///
/// Без суффикса (по умолчанию) — text. С суффиксом — указанное действие.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockMode {
    /// Текст в stdin окна (auto-launch если не существует)
    Text,
    /// Закрыть окно
    Close,
    /// Записать содержимое блока в файл напрямую (минуя shell)
    Write,
}

impl std::str::FromStr for BlockMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "close" => Ok(Self::Close),
            "text" => Ok(Self::Text),
            "write" => Ok(Self::Write),
            _ => Err(format!("unknown block mode: {s}")),
        }
    }
}

impl std::fmt::Display for BlockMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockMode::Text => f.write_str("text"),
            BlockMode::Close => f.write_str("close"),
            BlockMode::Write => f.write_str("write"),
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
