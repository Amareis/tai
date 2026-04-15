#![cfg(test)]

use tai::agent::{AgentResponse, TestAgent};
use tai::session::SessionDir;
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

fn test_session() -> SessionDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_path_buf();
    std::fs::write(project.join("tai.md"), "").ok();
    SessionDir::create_new(&project, false).expect("session")
}

fn test_server(session: SessionDir, agent: TestAgent) -> Server {
    let cwd = session.workspace().to_path_buf();
    Server::with_backend(session, Box::new(LocalBackend::new(cwd)), Box::new(agent))
}

#[tokio::test]
#[traced_test]
async fn tick_empty_session() {
    let session = test_session();
    let agent = TestAgent::new().add_step(
        |_state: &State| {},
        AgentResponse::new(),
    );

    let mut server = test_server(session, agent);
    let mut segments = vec![];
    server.tick(&mut segments).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn tick_agent_runs_command() {
    let session = test_session();
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
                let found = state.outputs.get("build")
                    .is_some_and(|o| o.stdout.contains("marker-xyz"));
                assert!(found, "expected marker-xyz in outputs, got: {:?}", state.outputs);
            },
            AgentResponse::block("build", BlockMode::Close, ""),
        );

    let mut server = test_server(session, agent);
    let mut segments = vec![];
    server.tick(&mut segments).await.unwrap();
    server.tick(&mut segments).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn close_removes_from_tracked() {
    let session = test_session();
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
            AgentResponse::new(),
        );

    let mut server = test_server(session, agent);
    let mut segments = vec![];
    server.tick(&mut segments).await.unwrap();
    assert_eq!(segments.len(), 1);
    server.tick(&mut segments).await.unwrap();
    assert_eq!(segments.len(), 0);
    server.tick(&mut segments).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn exec_runs_once_then_caches() {
    let session = test_session();
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("install", BlockMode::Exec, "echo installed-once"),
        )
        .add_step(
            |state: &State| {
                let found = state.outputs.get("install")
                    .is_some_and(|o| o.stdout.contains("installed-once"));
                assert!(found, "expected 'installed-once' in install output, got: {:?}", state.outputs);
            },
            AgentResponse::block("install", BlockMode::Close, ""),
        );

    let mut server = test_server(session, agent);
    let mut segments = vec![];
    server.tick(&mut segments).await.unwrap();
    server.tick(&mut segments).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn upsert_replaces_existing_title() {
    let session = test_session();
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
                assert_eq!(state.segments.len(), 1, "expected 1 segment, got {}", state.segments.len());
                let found = state.outputs.get("build")
                    .is_some_and(|o| o.stdout.contains("second") && !o.stdout.contains("first"));
                assert!(found, "expected 'second' but not 'first' in build output, got: {:?}", state.outputs);
            },
            AgentResponse::block("build", BlockMode::Close, ""),
        );

    let mut server = test_server(session, agent);
    let mut segments = vec![];
    server.tick(&mut segments).await.unwrap();
    assert_eq!(segments.len(), 1);
    server.tick(&mut segments).await.unwrap();
    assert_eq!(segments.len(), 1);
    server.tick(&mut segments).await.unwrap();
    assert_eq!(segments.len(), 0);

    assert_test_agent(&server);
}
