use futures_util::FutureExt;
use futures_util::stream::StreamExt;
mod client;
mod connection;
pub mod model_view;
mod utils;

pub use client::Client;

use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::layout::Rect;
use ratatui::prelude::{Color, Style};
use ratatui::widgets::Paragraph;
use std::path::PathBuf;
use std::time::Duration;

use crate::backend::TerminalBackend;
use connection::Connection;
use rustyline_async::ReadlineError;
use thiserror::Error;
use tokio::net::UnixListener;
use tokio::select;
use tokio::time::sleep;
use tracing::info;
use utils::RmFileOnDrop;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("connection closed")]
    ConnectionClosed,

    #[error("readline error: {0}")]
    Readline(#[from] ReadlineError),
}

pub struct Server<Back: TerminalBackend> {
    client: Connection,
    tui: DefaultTerminal,
    back: Back,
}

pub async fn bind(socket_path: &PathBuf) -> Result<(UnixListener, RmFileOnDrop), CoreError> {
    if socket_path.exists() {
        tokio::fs::remove_file(&socket_path).await?;
    }

    let listener = UnixListener::bind(socket_path)?;
    info!("listening on {}", socket_path.display());

    Ok((listener, RmFileOnDrop::new(socket_path.clone())))
}

impl<Back: TerminalBackend> Server<Back> {
    pub async fn accept(
        tui: DefaultTerminal,
        back: Back,
        unix_listener: &UnixListener,
    ) -> Result<Self, CoreError> {
        Connection::accept(unix_listener)
            .await
            .map(|client| Self { client, tui, back })
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        let Server { client, .. } = self;

        client
            .write_line("════════════════════════════════════════════════════════════")
            .await?;
        client
            .write_line("STATE: Active 0 | Frozen 0 | Tokens: 0")
            .await?;
        client
            .write_line("════════════════════════════════════════════════════════════")
            .await?;
        client.write_line("").await?;
        client
            .write_line("TAI Server ready. Type commands in this window.")
            .await?;
        client.write_line("").await?;

        info!("model viewport initialized");

        self.tui_loop().await
    }

    async fn tui_loop(&mut self) -> Result<(), CoreError> {
        let Server { client, tui, back } = self;

        let mut exit = false;

        let mut events = EventStream::new();

        while !exit {
            tui.draw(|f| {
                let area = Rect::new(0, 0, f.area().width, f.area().height);
                f.render_widget(
                    Paragraph::new("TAI Server (User Viewport)\n\nPress ESC or Ctrl+C to exit")
                        .style(Style::default().fg(Color::Green)),
                    area,
                );
            })?;
            let event = events.next().fuse();
            select! {
                tui_event = event => {
                    match tui_event {
                        Some(Ok(Event::Key(key))) => {
                            if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                                exit = true;
                            }
                            if key.code == KeyCode::Esc {
                                exit = true;
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => tracing::error!("Tui events error: {:?}", e),
                        None => break,
                    }
                }
                should_exit = client_loop(client, back) => {
                    match should_exit {
                        Ok(e) => {exit = e}
                        Err(e) => {
                            tracing::error!("error reading from model channel: {}", e);
                            return Err(e)
                        }
                    }
                }
                () = sleep(Duration::from_millis(60)) => {}
            }
        }
        Ok(())
    }
}

async fn client_loop(
    client: &mut Connection,
    _back: &mut impl TerminalBackend,
) -> Result<bool, CoreError> {
    if let Some(line) = client.read_line().await? {
        info!("received from model channel: {}", line);
        client.write_line(&format!("Echo: {line}")).await?;
        Ok(line == "exit")
    } else {
        info!("model channel closed");
        Err(CoreError::ConnectionClosed)
    }
}
