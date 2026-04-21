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

async fn test_session() -> SessionDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().to_path_buf();
    std::fs::write(project.join("tai.md"), "").ok();
    SessionDir::create_new(&project, Some("test")).await.expect("session")
}

fn test_server(session: SessionDir, agent: TestAgent) -> Server {
    let cwd = session.workspace().to_path_buf();
    Server::new(session, Box::new(LocalBackend::new(cwd)), Box::new(agent))
}

fn task(steps: &str) -> AgentResponse {
    AgentResponse::new().with_task(steps)
}

#[tokio::test]
#[traced_test]
async fn tick_empty_session() {
    let session = test_session().await;
    let agent = TestAgent::new().add_step(
        |_state: &State| {},
        task("done"),
    );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn tick_agent_runs_command() {
    let session = test_session().await;
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo marker-xyz")
                .with_task("check build output"),
        )
        .add_step(
            |state: &State| {
                let found = state.outputs.get("build")
                    .is_some_and(|o| o.stdout.contains("marker-xyz"));
                assert!(found, "expected marker-xyz in outputs, got: {:?}", state.outputs);
            },
            AgentResponse::block("build", BlockMode::Close, "")
                .with_task("closing build"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    let mut resp = AgentResponse::new();
    resp = server.tick_tack(&mut state, resp).await.unwrap();
    assert_eq!(state.segments.len(), 0);
    server.tick_tack(&mut state, resp).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn close_removes_from_tracked() {
    let session = test_session().await;
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo hello")
                .with_task("watching build"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Close, "")
                .with_task("closing"),
        )
        .add_step(
            |_state: &State| {},
            task("done"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.segments.len(), 0);
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.segments.len(), 1);
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.segments.len(), 0);

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn exec_runs_once_then_caches() {
    let session = test_session().await;
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("install", BlockMode::Exec, "echo installed-once")
                .with_task("installing"),
        )
        .add_step(
            |state: &State| {
                let found = state.outputs.get("install")
                    .is_some_and(|o| o.stdout.contains("installed-once"));
                assert!(found, "expected 'installed-once' in install output, got: {:?}", state.outputs);
            },
            AgentResponse::block("install", BlockMode::Close, "")
                .with_task("closing"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.segments.len(), 0);
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn upsert_replaces_existing_title() {
    let session = test_session().await;
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo first")
                .with_task("step 1"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo second")
                .with_task("step 2"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.segments.len(), 1, "expected 1 segment, got {}", state.segments.len());
                let found = state.outputs.get("build")
                    .is_some_and(|o| o.stdout.contains("second") && !o.stdout.contains("first"));
                assert!(found, "expected 'second' but not 'first' in build output, got: {:?}", state.outputs);
            },
            AgentResponse::block("build", BlockMode::Close, "")
                .with_task("closing"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.segments.len(), 0);
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.segments.len(), 1);
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn write_mode_creates_file() {
    let session = test_session().await;
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("test-file.txt", BlockMode::Write, "hello world")
                .with_task("writing file"),
        )
        .add_step(
            |state: &State| {
                let output = state.outputs.get("test-file.txt");
                assert!(output.is_some(), "expected test-file.txt in outputs");
                let stdout = &output.unwrap().stdout;
                assert!(stdout.contains("hello world"), "expected 'hello world' in output, got: {stdout}");
            },
            AgentResponse::block("test-file.txt", BlockMode::Close, "")
                .with_task("closing"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.segments.len(), 0);
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn edit_mode_modifies_file() {
    let session = test_session().await;

    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("edit-test.txt", BlockMode::Write, "line one\nline two\nline three")
                .with_task("creating file"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("edit-test.txt", BlockMode::Edit(Vec::new()), "Change Exactly L2:line two\nREPLACED\n.")
                .with_task("editing file"),
        )
        .add_step(
            |state: &State| {
                let output = state.outputs.get("edit-test.txt");
                assert!(output.is_some(), "expected edit-test.txt in outputs");
                let stdout = &output.unwrap().stdout;
                assert!(stdout.contains("REPLACED"), "expected 'REPLACED' in output, got: {stdout}");
                assert!(!stdout.contains("line two"), "should not contain 'line two', got: {stdout}");
            },
            AgentResponse::block("edit-test.txt", BlockMode::Close, "")
                .with_task("closing"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn edit_mode_rejects_unknown_line() {
    let session = test_session().await;

    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("edit-test.txt", BlockMode::Write, "line one\nline two\nline three")
                .with_task("creating file"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("edit-test.txt", BlockMode::Edit(Vec::new()), "Change Exactly L2:wrong line\nREPLACED\n.")
                .with_task("editing file with bad line ref"),
        )
        .add_step(
            |state: &State| {
                let output = state.outputs.get("edit-test.txt");
                assert!(output.is_some(), "expected edit-test.txt in outputs");
                let stdout = &output.unwrap().stdout;
                assert!(stdout.contains("edit error:"), "expected edit error in output, got: {stdout}");
                assert!(!stdout.contains("REPLACED"), "should not contain 'REPLACED', got: {stdout}");
            },
            AgentResponse::block("edit-test.txt", BlockMode::Close, "")
                .with_task("closing"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn file_mode_shows_file() {
    let session = test_session().await;

    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("file-test.txt", BlockMode::Write, "file content here")
                .with_task("creating file"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("file-test.txt", BlockMode::File, "")
                .with_task("viewing file"),
        )
        .add_step(
            |state: &State| {
                let output = state.outputs.get("file-test.txt");
                assert!(output.is_some(), "expected file-test.txt in outputs");
                assert!(output.unwrap().stdout.contains("file content here"));
            },
            AgentResponse::block("file-test.txt", BlockMode::Close, "")
                .with_task("closing"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn task_persists_and_completes() {
    let session = test_session().await;
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo ok")
                .with_task("check build"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.task, "check build");
            },
            {
                let mut resp = AgentResponse::new().with_task("build ok");
                resp.complete = true;
                resp
            },
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.task, "check build");
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert!(state.is_complete());
    assert_eq!(state.task, "build ok");

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn mind_persists_across_ticks() {
    let session = test_session().await;
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo ok")
                .with_task("step 1: check build"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.task, "step 1: check build");
            },
            AgentResponse::block("build", BlockMode::Close, "")
                .with_task("step 2: done"),
        );

    let mut server = test_server(session, agent);
    let mut state = State::default();
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();
    assert_eq!(state.task, "step 1: check build");
    server.tick_tack(&mut state, AgentResponse::new()).await.unwrap();

    assert_test_agent(&server);
}
