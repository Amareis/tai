pub mod backend;
pub mod config;
pub mod fd;
pub mod types;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;
use std::env;
use std::fs::File;
use std::io::Write;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use std::path::PathBuf;
use std::time::Duration;
use crossterm::event::{read, Event, KeyCode, KeyModifiers};
use ratatui::{init, restore, DefaultTerminal};
use tokio::sync::oneshot;
use tokio::time::sleep;
use tracing_subscriber::EnvFilter;

/// Запуск TAI сервера: User viewport + Kitty + Model viewport.
pub async fn run_server(
    socket_path: Option<PathBuf>,
    hidden: bool,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    let mut tm = init();

    tracing::info!("user viewport initialized");

    let uuid = uuid::Uuid::new_v4();
    let kitty_socket = env::temp_dir().join(format!("tai-kitty-{}.sock", &uuid));

    let fd_socket = socket_path
        .unwrap_or_else(|| env::temp_dir().join(PathBuf::from(format!("tai-{}.sock", &uuid))));

    // Создаем канал: tx (отправитель), rx (получатель)
    let (tx, rx) = oneshot::channel();
    fd::recv_connection(fd_socket.clone(), tx);

    let bin_path = env::current_exe()?;

    let args = vec![
        if debug { "--hold" } else { "" },
        bin_path.to_str().ok_or("invalid path")?,
        "client",
        if debug { "--debug" } else { "" },
        "--socket",
        fd_socket.to_str().ok_or("invalid socket path")?,
    ];
    let _kitty =
        backend::kitty::KittyBackend::spawn(&args, Some(kitty_socket.clone()), hidden).await?;

    tracing::info!("kitty spawned, socket: {}", kitty_socket.display());

    let mut model_file = rx.await??;
    print_hello(&mut model_file);

    tracing::info!("model viewport initialized, drawing...");

    let _ = draw_hello(&mut tm).await;

    restore();
    Ok(())
}

fn print_hello(out: &mut File) {
    let _ = out.write_all("Model viewport\n".repeat(100).as_bytes());
}

/// Запуск TAI клиента внутри Kitty: передать FD и спать.
pub async fn run_client(
    socket_path: PathBuf,
    _debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    tracing::info!("tai client starting, socket: {}", socket_path.display());
    fd::send_connection(socket_path).await?;
    let stdin = io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    print!("Введите текст (exit для выхода):\n> ");

    // Читаем строки в цикле по мере их поступления
    while let Some(line) = lines.next_line().await? {
        print!("Эхо: {}\n> ", line);

        if line == "exit" {
            break;
        }
    }

    // tokio::signal::ctrl_c().await?;

    Ok(())
}

async fn draw_hello(tm: &mut DefaultTerminal) -> Result<(), Box<dyn std::error::Error>> {
    let mut exit = false;
    while !exit {
        tm.draw(|f| {
            let area = Rect::new(0, 0, f.area().width, f.area().height);
            f.render_widget(
                Paragraph::new("TAI Server (User Viewport)\n".repeat(5))
                    .style(Style::default().fg(Color::Green)),
                area,
            );
        })?;
        match read()? {
            Event::Key(key) => {
                if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                    exit = true;
                }
                if key.code == KeyCode::Esc {
                    exit = true;
                }
            }
            Event::Mouse(_mouse_event) => {

            }
            _ => {}
        }
        sleep(Duration::from_millis(60)).await;
    }
    Ok(())
}
