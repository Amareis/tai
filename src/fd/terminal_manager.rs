use std::collections::HashMap;
use std::fs::File;
use std::{io};
use std::os::fd::{AsRawFd, FromRawFd};

use crossterm::event::DisableMouseCapture;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{ExecutableCommand};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, restore};
use thiserror::Error;

use super::ViewId;

#[derive(Debug, Error)]
pub enum TerminalManagerError {
    #[error("terminal error: {0}")]
    Terminal(#[from] io::Error),

    #[error("viewport not found: {0:?}")]
    NotFound(ViewId),

    #[error("viewport already registered: {0:?}")]
    AlreadyExists(ViewId),
}

type Term = Terminal<CrosstermBackend<File>>;

/// Управляет ratatui Terminal для каждого viewport.
///
/// User viewport — stdout сервера, Model viewport — FD от Kitty клиента.
/// Каждый Terminal создаётся на своём файловом дескрипторе.
pub struct TerminalManager {
    terminals: HashMap<ViewId, Term>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            terminals: HashMap::new(),
        }
    }

    /// Создать Terminal на stdout (User viewport).
    pub fn init_user_viewport(&mut self, debug: bool) -> Result<(), TerminalManagerError> {
        if !debug {
            enable_raw_mode()?;
        }
        set_panic_hook();

        let raw_fd = io::stdout().as_raw_fd();
        let file = unsafe { File::from_raw_fd(raw_fd) };
        let mut backend = CrosstermBackend::new(file);
        backend.execute(EnterAlternateScreen)?;

        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        self.terminals.insert(ViewId::User, terminal);
        Ok(())
    }

    /// Создать Terminal на FD от Kitty клиента (Model viewport).
    pub fn init_model_viewport(&mut self, file: File) -> Result<(), TerminalManagerError> {
        let backend = CrosstermBackend::new(file);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        self.terminals.insert(ViewId::Model, terminal);
        Ok(())
    }

    /// Получить mutable ref на Terminal для viewport.
    pub fn get_mut(&mut self, id: ViewId) -> Result<&mut Term, TerminalManagerError> {
        self.terminals
            .get_mut(&id)
            .ok_or(TerminalManagerError::NotFound(id))
    }

    /// Resize конкретного viewport.
    pub fn resize(
        &mut self,
        id: ViewId,
        width: u16,
        height: u16,
    ) -> Result<(), TerminalManagerError> {
        let term = self.get_mut(id)?;
        term.resize(ratatui::layout::Rect::new(0, 0, width, height))?;
        Ok(())
    }

    /// Восстановить терминалы при shutdown.
    pub fn restore(&mut self) {
        if let Err(e) = disable_raw_mode() {
            tracing::warn!("failed to disable raw mode for main terminal: {e}");
        }

        for (_, terminal) in self.terminals.iter_mut() {
            let backend = terminal.backend_mut();
            let _ = backend.execute(LeaveAlternateScreen);
            let _ = backend.execute(DisableMouseCapture);
        }
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Sets a panic hook that restores the terminal before panicking.
///
/// Replaces the panic hook with a one that will restore the terminal state before calling the
/// original panic hook. This ensures that the terminal is left in a good state when a panic occurs.
fn set_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
}
