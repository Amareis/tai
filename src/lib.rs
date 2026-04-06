pub mod backend;
pub mod config;
pub mod core;
pub mod types;

use crate::core::{Client, Server, bind};
use ratatui::{init, restore};
use std::env;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

struct RestoreTerm;

impl Drop for RestoreTerm {
    fn drop(&mut self) {
        restore();
    }
}

/// Запуск TAI сервера: User viewport + Kitty + Model viewport.
pub async fn run_server(
    socket_path: Option<PathBuf>,
    hidden: bool,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    let tm = init();
    let _restore = RestoreTerm;

    let uuid = uuid::Uuid::new_v4();
    let kitty_socket = env::temp_dir().join(format!("tai-kitty-{}.sock", &uuid));

    let client_socket =
        socket_path.unwrap_or_else(|| env::temp_dir().join(format!("tai-{}.sock", &uuid)));

    let (listener, _del) = bind(&client_socket).await?;

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

    let kitty = backend::kitty::KittyBackend::spawn(&args_refs, &kitty_socket, hidden).await?;

    tracing::info!("kitty spawned, socket: {}", kitty_socket.display());

    let mut server = Server::accept(tm, kitty, &listener).await?;
    let _ = server.run().await;

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

    let mut client = Client::connect(socket_path).await?;
    client.run().await?;
    Ok(())
}
