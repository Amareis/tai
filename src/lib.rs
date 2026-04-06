pub mod backend;
pub mod client;
pub mod config;
pub mod types;

use crossterm::event::{Event, KeyCode, KeyModifiers, read};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;
use ratatui::{init, restore};
use std::env;
use std::path::PathBuf;
use std::time::Duration;
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

    let client_socket =
        socket_path.unwrap_or_else(|| env::temp_dir().join(format!("tai-{}.sock", &uuid)));

    let server = client::Server::bind(client_socket.clone()).await?;

    let bin_path = env::current_exe()?;

    let mut args: Vec<String> = vec![
        "--hold".to_string(),
        bin_path.to_str().ok_or("invalid path")?.to_string(),
        "client".to_string(),
        "--socket".to_string(),
        client_socket
            .to_str()
            .ok_or("invalid socket path")?
            .to_string(),
    ];

    if debug {
        args.push("--debug".to_string());
    }

    let args_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let _kitty =
        backend::kitty::KittyBackend::spawn(&args_refs, Some(kitty_socket.clone()), hidden).await?;

    tracing::info!("kitty spawned, socket: {}", kitty_socket.display());

    let (mut conn, socket_path) = server.accept().await?;
    tracing::info!("model client connected from {}", socket_path.display());

    conn.write_line("════════════════════════════════════════════════════════════")
        .await?;
    conn.write_line("STATE: Active 0 | Frozen 0 | Tokens: 0")
        .await?;
    conn.write_line("════════════════════════════════════════════════════════════")
        .await?;
    conn.write_line("").await?;
    conn.write_line("TAI Server ready. Type commands in this window.")
        .await?;
    conn.write_line("").await?;
    conn.write("> ").await?;

    tracing::info!("model viewport initialized");

    tokio::spawn(async move {
        loop {
            match conn.read_line().await {
                Ok(Some(line)) => {
                    tracing::info!("received from model channel: {}", line);
                    let _ = conn.write_line(&format!("Echo: {line}")).await;
                    let _ = conn.write_line("> ").await;
                }
                Ok(None) => {
                    tracing::info!("model channel closed");
                    break;
                }
                Err(e) => {
                    tracing::error!("error reading from model channel: {}", e);
                    break;
                }
            }
        }
    });

    draw_hello(&mut tm)?;

    restore();
    Ok(())
}

/// Запуск TAI клиента внутри Kitty: текстовый протокол через Unix socket.
pub async fn run_client(
    socket_path: PathBuf,
    _debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    tracing::info!("tai client starting, socket: {}", socket_path.display());
    client::run_client(socket_path).await?;
    Ok(())
}

fn draw_hello(tm: &mut ratatui::DefaultTerminal) -> Result<(), Box<dyn std::error::Error>> {
    let mut exit = false;
    while !exit {
        tm.draw(|f| {
            let area = Rect::new(0, 0, f.area().width, f.area().height);
            f.render_widget(
                Paragraph::new("TAI Server (User Viewport)\n\nPress ESC or Ctrl+C to exit")
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
            Event::Mouse(_) => exit = false,
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    Ok(())
}
