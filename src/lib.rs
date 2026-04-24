pub mod agent;
pub mod backend;
pub mod core;
pub mod response;
pub mod session;
pub mod state;
pub mod types;

use crate::agent::{Agent, LlmAgent};
use crate::core::Server;
use crate::session::SessionDir;
use std::env;
use std::path::Path;
use tracing::error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

pub fn create_server(
    session: SessionDir,
    agent: Box<dyn Agent>,
) -> Result<Server, Box<dyn std::error::Error>> {
    let cwd = session.workspace().to_path_buf();
    let back = Box::new(backend::local::LocalBackend::new(cwd));
    Ok(Server::new(session, back, agent))
}

pub async fn run_server(
    session_path: Option<&Path>,
    task: Option<&str>,
    debug: bool,
    max_ticks: Option<u64>,
    tui: bool,
    no_delegate: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_dir = env::current_dir()?;
    let session = SessionDir::create_or_open(session_path, &project_dir, task).await?;

    let level = if debug { "tai=debug" } else { "tai=info" };
    let filter = EnvFilter::from_default_env().add_directive(level.parse()?);
    let log_path = session.internal_path().join("tai.log");
    let log_path_clone = log_path.clone();
    let file_writer = move || -> Box<dyn std::io::Write + Send> {
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path_clone)
        {
            Ok(f) => Box::new(f),
            Err(_) => Box::new(std::io::sink()),
        }
    };
    if tui {
        let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(file_writer)
            .with_ansi(false);
        tracing_subscriber::registry()
            .with(filter)
            .with(stdout_layer)
            .with(file_layer)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(file_writer)
            .with_ansi(false)
            .init();
    }

    let model = env::var("OPENAI_MODEL")?;
    let mut agent = Box::new(LlmAgent::new(model));
    agent.tui = tui;
    let mut server = create_server(session, agent)?;
    server.debug = debug;
    server.max_ticks = max_ticks;
    server.tui = tui;
    server.no_delegate = no_delegate;

    if let Err(e) = server.run().await {
        error!("Server run error: {}", e);
    }
    Ok(())
}
