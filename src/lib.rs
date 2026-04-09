pub mod agent;
pub mod backend;
pub mod config;
pub mod core;
pub mod prompt;
pub mod response;
pub mod routing;
pub mod types;

use crate::agent::{Agent, NopAgent};
use crate::core::connection::Connection;
use crate::core::{Client, Server, utils::bind};
use std::env;
use std::path::PathBuf;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

/// Создание TAI сервера: User viewport + Kitty + Model viewport.
pub async fn create_server(
    socket_path: Option<PathBuf>,
    agent: Box<dyn Agent>,
    hidden: bool,
) -> Result<Server, Box<dyn std::error::Error>> {
    let uuid = uuid::Uuid::new_v4();
    let kitty_socket = env::temp_dir().join(format!("tai-kitty-{}.sock", &uuid));

    let client_socket =
        socket_path.unwrap_or_else(|| env::temp_dir().join(format!("tai-{}.sock", &uuid)));

    let (listener, guard) = bind(&client_socket).await?;

    let mut args: Vec<String> = vec![
        "--hold".to_string()
    ];

    // if cfg!(test) {
        args.extend([
            "nc".to_string(),
            "-U".to_string(),
            client_socket
                .to_str()
                .ok_or("invalid socket path")?
                .to_string(),
        ]);
    // } else {
    //     let bin_path = env::current_exe()?;
    //     args.extend([
    //         bin_path.to_str().ok_or("invalid path")?.to_string(),
    //         "client".to_string(),
    //         "--socket".to_string(),
    //         client_socket
    //             .to_str()
    //             .ok_or("invalid socket path")?
    //             .to_string(),
    //     ]);
    // }

    let args_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let kitty =
        Box::new(backend::kitty::KittyBackend::spawn(&args_refs, &kitty_socket, hidden).await?);

    tracing::info!("kitty spawned, socket: {}", kitty_socket.display());

    let client = Connection::accept(&listener, Some(guard), Some(Duration::from_secs(1))).await?;

    Ok(Server::new(client, kitty, agent))
}

/// Запуск TAI сервера: User viewport + Kitty + Model viewport.
pub async fn run_server(
    socket_path: Option<PathBuf>,
    hidden: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();
    let agent = Box::new(NopAgent);
    let mut server = create_server(socket_path, agent, hidden).await?;
    let _ = server.run().await;
    Ok(())
}

/// Запуск TAI клиента внутри Kitty: текстовый протокол через Unix socket.
pub async fn run_client(socket_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    let mut client = Client::connect(socket_path).await?;
    client.run().await?;
    Ok(())
}
