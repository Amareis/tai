use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Состояние терминального окна.
///
/// Жизненный цикл: `Active` → `Frozen` → `Archived`.
///
/// - **Active** — PTY-процесс жив, окно видно в терминале. В промпте если focused.
/// - **Frozen** — процесс завершён, вывод захвачен в RAM. Есть exit code.
/// - **Archived** — содержимое сброшено на диск, из контекста убрано.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WindowState {
    /// Окно с живым PTY-процессом.
    Active {
        /// Идентификатор окна в backend (kitty window-id, tmux pane-id и т.д.)
        backend_id: String,
        /// PID процесса внутри окна
        pid: u32,
        /// Заголовок окна
        title: String,
    },
    /// Завершённое окно — снимок вывода в памяти.
    Frozen {
        /// Полный захваченный вывод
        content: String,
        /// Код завершения процесса
        exit_code: i32,
        /// Время захвата (freeze)
        captured_at: DateTime<Utc>,
    },
    /// Архивированное окно — содержимое на диске.
    Archived {
        /// Путь к файлу с захваченным выводом
        file_path: PathBuf,
        /// Код завершения процесса
        exit_code: i32,
    },
}

impl WindowState {
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, WindowState::Active { .. })
    }

    #[must_use]
    pub fn is_frozen(&self) -> bool {
        matches!(self, WindowState::Frozen { .. })
    }

    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            WindowState::Active { .. } => None,
            WindowState::Frozen { exit_code, .. } | WindowState::Archived { exit_code, .. } => {
                Some(*exit_code)
            }
        }
    }
}

/// Терминальное окно — объект внимания модели.
///
/// Каждое окно привязано к терминальной сессии. Модель «видит» содержимое
/// развёрнутых (focused) окон и может отправлять в них команды.
/// Служит одновременно и как модель данных, и как элемент контекста модели.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Window {
    /// Уникальный идентификатор окна (генерируется при создании)
    pub id: String,
    /// Текущее состояние окна
    pub state: WindowState,
    /// Развёрнуто ли окно в контексте модели (focused = полный текст в промпте)
    pub focused: bool,
    /// Опциональная сводка содержимого (для свёрнутых окон)
    pub summary: Option<String>,
    /// Пользовательские теги для организации окон
    pub tags: Vec<String>,
}

impl Window {
    #[must_use]
    pub fn new_active(backend_id: String, pid: u32, title: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            state: WindowState::Active {
                backend_id,
                pid,
                title,
            },
            focused: false,
            summary: None,
            tags: Vec::new(),
        }
    }
}

/// Рабочая сессия — набор окон + путь к mind-файлу.
///
/// Session aggregates all terminal windows belonging to one work context.
/// The mind file stores the model's ongoing thoughts and goals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    /// Уникальный идентификатор сессии
    pub id: String,
    /// Все окна в сессии
    pub windows: Vec<Window>,
    /// Путь к mind-файлу (цели, размышления модели)
    pub mind_path: PathBuf,
}

impl Session {
    #[must_use]
    pub fn new(id: String, mind_path: PathBuf) -> Self {
        Self {
            id,
            windows: Vec::new(),
            mind_path,
        }
    }

    pub fn active_windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter().filter(|w| w.state.is_active())
    }

    pub fn focused_windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter().filter(|w| w.focused)
    }

    #[must_use]
    pub fn find_window(&self, id: &str) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn find_window_mut(&mut self, id: &str) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }
}

/// Триггер нового тика (цикла ядра).
///
/// Тик — один проход assemble → invoke → parse → execute → wait.
/// Триггер определяет, что запускает новый тик.
/// TODO: Phase 6 — использовать в kernel/mod.rs
#[derive(Debug, Clone, PartialEq)]
pub enum TickTrigger {
    /// Терминальное окно завершилось (`at_prompt` == true)
    WindowExited { window_id: String, exit_code: i32 },
    /// Пользователь написал сообщение в TUI chat
    UserMessage,
    /// Прошёл idle-timeout без событий
    IdleTimeout,
}
