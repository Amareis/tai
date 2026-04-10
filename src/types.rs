use serde::{Deserialize, Serialize};
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

/// Сегмент ответа модели после парсинга.
///
/// Блок с `window == "tai"` обрабатывается через clap-парсер в ядре,
/// а не отправляется как текст в окно. Остальные блоки — send-text
/// (auto-launch если окно не существует).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParsedSegment {
    /// Code block: ` ```title\ncontent\n``` `
    Block { window: String, content: String },
    /// Текст вне блоков — prose, сохраняется для контекста
    Prose(String),
}
