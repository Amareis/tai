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
use tracing_subscriber::EnvFilter;

pub fn create_server(
    session: SessionDir,
    agent: Box<dyn Agent>,
) -> Result<Server, Box<dyn std::error::Error>> {
    let cwd = session.workspace().to_path_buf();
    let back = Box::new(backend::local::LocalBackend::new(cwd));
    Ok(Server::with_backend(session, back, agent))
}

pub async fn run_server(
    session_path: Option<&Path>,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();

    let project_dir = env::current_dir()?;
    let session = SessionDir::create_or_open(session_path, &project_dir, debug)?;

    let model = env::var("OPENAI_MODEL")?;
    let mut agent = Box::new(LlmAgent::new(model));
    agent.debug = debug;
    let mut server = create_server(session, agent)?;
    server.debug = debug;

    if let Err(e) = server.run().await {
        error!("Server run error: {}", e);
    }
    Ok(())
}
