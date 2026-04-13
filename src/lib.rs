pub mod agent;
pub mod backend;
pub mod core;
pub mod state;
pub mod response;
pub mod types;

use crate::agent::{Agent, LlmAgent};
use crate::core::Server;
use std::env;
use tracing::error;
use tracing_subscriber::EnvFilter;

pub fn create_server(
    agent: Box<dyn Agent>,
) -> Result<Server, Box<dyn std::error::Error>> {
    let back = Box::new(backend::local::LocalBackend::new());
    Ok(Server::new(back, agent))
}

pub async fn run_server(
    debug: bool,
    initial: &[(&str, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tai=info".parse()?))
        .init();
    let model = env::var("OPENAI_MODEL")?;
    let mut agent = Box::new(LlmAgent::new(model));
    agent.debug = debug;
    let mut server = create_server(agent)?;
    server.debug = debug;
    if let Err(e) = server.run(initial).await {
        error!("Server run error: {}", e);
    }
    Ok(())
}
