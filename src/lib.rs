pub mod backend;
pub mod config;
pub mod fd;
pub mod types;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use fd::ViewId;
use fd::terminal_manager::TerminalManager;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Paragraph, Wrap};
use std::env;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;
use tokio::sync::oneshot;
/// Запуск TAI сервера: User viewport + Kitty + Model viewport.
pub async fn run_server(
    socket_path: Option<PathBuf>,
    hidden: bool,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    let mut tm = TerminalManager::new();
    tm.init_user_viewport(debug)?;

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

    tracing::info!(
        "kitty spawned, socket: {}",
        kitty_socket.display()
    );

    let model_file = rx.await??;
    tm.init_model_viewport(model_file)?;

    tracing::info!("model viewport initialized, drawing...");

    draw_hello(&mut tm)?;

    tracing::info!("press Ctrl+C to exit");
    tokio::signal::ctrl_c().await?;

    tm.restore();
    Ok(())
}


/// Запуск TAI клиента внутри Kitty: передать FD и спать.
pub async fn run_client(socket_path: PathBuf, debug: bool) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    //todo что делать с рав модом на дочернем терминале?
    if !debug {
        let _ = enable_raw_mode();
    }
    tracing::info!("tai client starting, socket: {}", socket_path.display());
    fd::send_connection(socket_path).await?;

    tokio::signal::ctrl_c().await?;

    //todo что делать с рав модом на дочернем терминале?
    let _ = disable_raw_mode();
    Ok(())
}

fn draw_hello(tm: &mut TerminalManager) -> Result<(), Box<dyn std::error::Error>> {
    let user_term = tm.get_mut(ViewId::User)?;
    user_term.draw(|f| {
        let area = Rect::new(0, 0, f.area().width, f.area().height);
        f.render_widget(
            Paragraph::new("TAI Server (User Viewport)\n".repeat(100)).style(Style::default().fg(Color::Green)),
            area,
        );
    })?;

    let model_term = tm.get_mut(ViewId::Model)?;
    model_term.draw(|f| {
        let area = Rect::new(0, 0, f.area().width, f.area().height);
        f.render_widget(
            Paragraph::new("TAI Model Workspace\n".repeat(100)).style(Style::default().fg(Color::Cyan)).wrap(Wrap{trim: false}),
            area,
        );
    })?;

    Ok(())
}
