pub mod agent;
pub mod backend;
pub mod config;
pub mod core;
pub mod prompt;
pub mod response;
pub mod types;

use crate::agent::{Agent, LlmAgent};
use crate::core::{Server};
use std::env;
use std::path::PathBuf;
use tracing::error;
use tracing_subscriber::EnvFilter;

/// Создание TAI сервера: User viewport + Kitty + Model viewport.
pub async fn create_server(
    _socket_path: Option<PathBuf>,
    agent: Box<dyn Agent>,
    hidden: bool,
) -> Result<Server, Box<dyn std::error::Error>> {
    let uuid = uuid::Uuid::new_v4();
    let kitty_socket = env::temp_dir().join(format!("tai-kitty-{}.sock", &uuid));

    let kitty =
        Box::new(backend::kitty::KittyBackend::spawn(&[], &kitty_socket, hidden).await?);

    tracing::info!("kitty spawned, socket: {}", kitty_socket.display());

    Ok(Server::new(kitty, agent))
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
    let model = env::var("OPENAI_MODEL")?;
    let mut agent = Box::new(LlmAgent::new(model));
    agent.debug = debug;
    let mut server = create_server(socket_path, agent, hidden).await?;
    server.debug = debug;
    if let Err(e) = server.run().await {
        error!("Server run error: {}", e);
    }
    Ok(())
}

