#![cfg(test)]

use tai::agent::{AgentResponse, TestAgent};
use tai::types::BlockMode;
use tai::core::Server;
use tai::backend::local::LocalBackend;
use tai::prompt::Prompt;
use tracing_test::traced_test;

fn assert_test_agent(server: &Server) {
    server.agent.as_any().expect("expected as_any in TestAgent")
        .downcast_ref::<TestAgent>()
        .expect("expected TestAgent").assert_all_consumed();
}

fn test_backend() -> LocalBackend {
    LocalBackend::new()
}

#[tokio::test]
#[traced_test]
async fn tick_empty_session() {
    let agent = TestAgent::new().add_step(
        |_prompt: &Prompt| {},
        AgentResponse::empty(),
    );

    let mut server = Server::new(Box::new(test_backend()), Box::new(agent));
    server.tick().await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn tick_agent_runs_command() {
    let agent = TestAgent::new()
        .add_step(
            |_prompt: &Prompt| {},
            AgentResponse::block(
                "build",
                BlockMode::Text,
                "echo marker-xyz",
            ),
        )
        .add_step(
            |prompt: &Prompt| {
                let found = prompt.tracked.iter().any(|v| v.output.contains("marker-xyz"));
                assert!(found, "expected marker-xyz in tracked output, got: {:?}", prompt.tracked);
            },
            AgentResponse::block("build", BlockMode::Close, ""),
        );

    let mut server = Server::new(Box::new(test_backend()), Box::new(agent));
    server.tick().await.unwrap();
    server.tick().await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn close_removes_from_tracked() {
    let agent = TestAgent::new()
        .add_step(
            |_prompt: &Prompt| {},
            AgentResponse::block("build", BlockMode::Text, "echo hello"),
        )
        .add_step(
            |_prompt: &Prompt| {},
            AgentResponse::block("build", BlockMode::Close, ""),
        )
        .add_step(
            |_prompt: &Prompt| {},
            AgentResponse::empty(),
        );

    let mut server = Server::new(Box::new(test_backend()), Box::new(agent));
    server.tick().await.unwrap(); // text → pending
    server.tick().await.unwrap(); // apply text, close → pending
    assert_eq!(server.tracked_count(), 1);
    server.tick().await.unwrap(); // apply close
    assert_eq!(server.tracked_count(), 0);

    assert_test_agent(&server);
}
