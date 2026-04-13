#![cfg(test)]

use tai::agent::{AgentResponse, TestAgent};
use tai::types::BlockMode;
use tai::core::Server;
use tai::backend::local::LocalBackend;
use tai::state::State;
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
        |_state: &State| {},
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
            |_state: &State| {},
            AgentResponse::block(
                "build",
                BlockMode::View,
                "echo marker-xyz",
            ),
        )
        .add_step(
            |state: &State| {
                let found = state.tracked.iter().any(|v| v.output.contains("marker-xyz"));
                assert!(found, "expected marker-xyz in tracked output, got: {:?}", state.tracked);
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
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::View, "echo hello"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Close, ""),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::empty(),
        );

    let mut server = Server::new(Box::new(test_backend()), Box::new(agent));
    server.tick().await.unwrap();
    server.tick().await.unwrap();
    assert_eq!(server.tracked_count(), 1);
    server.tick().await.unwrap();
    assert_eq!(server.tracked_count(), 0);

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn exec_runs_once_then_caches() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("install", BlockMode::Exec, "echo installed-once"),
        )
        .add_step(
            |state: &State| {
                let found = state.tracked.iter().any(|v| {
                    v.title == "install" && v.output.contains("installed-once")
                });
                assert!(found, "expected 'installed-once' in install view, got: {:?}", state.tracked);
            },
            AgentResponse::block("install", BlockMode::Close, ""),
        );

    let mut server = Server::new(Box::new(test_backend()), Box::new(agent));
    server.tick().await.unwrap();
    server.tick().await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn upsert_replaces_existing_title() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::View, "echo first"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::View, "echo second"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.tracked.len(), 1, "expected 1 tracked view, got {}", state.tracked.len());
                let found = state.tracked.iter().any(|v| {
                    v.title == "build" && v.output.contains("second") && !v.output.contains("first")
                });
                assert!(found, "expected 'second' but not 'first' in build view, got: {:?}", state.tracked);
            },
            AgentResponse::block("build", BlockMode::Close, ""),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::empty(),
        );

    let mut server = Server::new(Box::new(test_backend()), Box::new(agent));
    server.tick().await.unwrap();
    assert_eq!(server.tracked_count(), 0);
    server.tick().await.unwrap();
    assert_eq!(server.tracked_count(), 1);
    server.tick().await.unwrap();
    assert_eq!(server.tracked_count(), 1);
    server.tick().await.unwrap();
    assert_eq!(server.tracked_count(), 0);

    assert_test_agent(&server);
}
