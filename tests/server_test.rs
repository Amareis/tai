#![cfg(test)]

use tai::agent::{AgentError, AgentResponse, TestAgent};
use tai::backend::local::LocalBackend;
use tai::core::{CoreError, Server};
use tai::session::SessionDir;
use tai::state::State;
use tai::types::BlockMode;
use tracing_test::traced_test;

struct TestRunner {
    _tmp: tempfile::TempDir,
    server: Server,
}

impl TestRunner {
    async fn new(agent: TestAgent) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project = tmp.path().to_path_buf();
        std::fs::write(project.join("tai.md"), "").ok();
        let session = SessionDir::create_new(&project, Some("test"))
            .await
            .expect("session");
        let cwd = session.workspace().to_path_buf();
        let server = Server::new(session, Box::new(LocalBackend::new(cwd)), Box::new(agent));
        Self { _tmp: tmp, server }
    }

    async fn run(mut self) {
        let mut state = State::default();
        let mut resp = None;
        loop {
            match self.server.tick_tack(&mut state, resp).await {
                Ok(None) => break,
                Ok(Some(new_resp)) => resp = Some(new_resp),
                Err(CoreError::Agent(AgentError::Api(msg)))
                    if msg.contains("no test step") =>
                {
                    break;
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        self.server
            .agent
            .as_any()
            .expect("expected as_any in TestAgent")
            .downcast_ref::<TestAgent>()
            .expect("expected TestAgent")
            .assert_all_consumed();
    }
}

fn task(steps: &str) -> AgentResponse {
    AgentResponse::new().with_task(steps)
}

#[tokio::test]
#[traced_test]
async fn tick_empty_session() {
    let agent = TestAgent::new().add_step(|_state: &State| {}, task("done"));
    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn tick_agent_runs_command() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo marker-xyz")
                .with_task("check build output"),
        )
        .add_step(
            |state: &State| {
                let found = state
                    .outputs
                    .get("build")
                    .is_some_and(|o| o.stdout.contains("marker-xyz"));
                assert!(
                    found,
                    "expected marker-xyz in outputs, got: {:?}",
                    state.outputs
                );
            },
            AgentResponse::block("build", BlockMode::Close, "").with_task("closing build"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn close_removes_from_tracked() {
    let agent = TestAgent::new()
        .add_step(
            |state: &State| {
                assert_eq!(state.segments.len(), 0);
            },
            AgentResponse::block("build", BlockMode::Watch, "echo hello")
                .with_task("watching build"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.segments.len(), 1);
            },
            AgentResponse::block("build", BlockMode::Close, "").with_task("closing"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.segments.len(), 0);
            },
            task("done"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn exec_runs_once_then_caches() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("install", BlockMode::Exec, "echo installed-once")
                .with_task("installing"),
        )
        .add_step(
            |state: &State| {
                let found = state
                    .outputs
                    .get("install")
                    .is_some_and(|o| o.stdout.contains("installed-once"));
                assert!(
                    found,
                    "expected 'installed-once' in install output, got: {:?}",
                    state.outputs
                );
            },
            AgentResponse::block("install", BlockMode::Close, "").with_task("closing"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn upsert_replaces_existing_title() {
    let agent = TestAgent::new()
        .add_step(
            |state: &State| {
                assert_eq!(state.segments.len(), 0);
            },
            AgentResponse::block("build", BlockMode::Watch, "echo first").with_task("step 1"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.segments.len(), 1);
            },
            AgentResponse::block("build", BlockMode::Watch, "echo second").with_task("step 2"),
        )
        .add_step(
            |state: &State| {
                assert_eq!(state.segments.len(), 1, "expected 1 segment, got {}", state.segments.len());
                let found = state
                    .outputs
                    .get("build")
                    .is_some_and(|o| o.stdout.contains("second") && !o.stdout.contains("first"));
                assert!(
                    found,
                    "expected 'second' but not 'first' in build output, got: {:?}",
                    state.outputs
                );
            },
            AgentResponse::block("build", BlockMode::Close, "").with_task("closing"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn write_mode_creates_file() {
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
                assert!(
                    stdout.contains("hello world"),
                    "expected 'hello world' in output, got: {stdout}"
                );
            },
            AgentResponse::block("test-file.txt", BlockMode::Close, "").with_task("closing"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn edit_mode_modifies_file() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block(
                "edit-test.txt",
                BlockMode::Write,
                "line one\nline two\nline three",
            )
            .with_task("creating file"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block(
                "edit-test.txt",
                BlockMode::Edit(None),
                "Change Exactly L2:line two\n<<'TAI'\nREPLACED\nTAI",
            )
            .with_task("editing file"),
        )
        .add_step(
            |state: &State| {
                let output = state.outputs.get("edit-test.txt");
                assert!(output.is_some(), "expected edit-test.txt in outputs");
                let stdout = &output.unwrap().stdout;
                assert!(
                    stdout.contains("REPLACED"),
                    "expected 'REPLACED' in output, got: {stdout}"
                );
                assert!(
                    !stdout.contains("line two"),
                    "should not contain 'line two', got: {stdout}"
                );
            },
            AgentResponse::block("edit-test.txt", BlockMode::Close, "").with_task("closing"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn edit_mode_rejects_unknown_line() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block(
                "edit-test.txt",
                BlockMode::Write,
                "line one\nline two\nline three",
            )
            .with_task("creating file"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block(
                "edit-test.txt",
                BlockMode::Edit(None),
                "Change Exactly L2:wrong line\n<<'TAI'\nREPLACED\nTAI",
            )
            .with_task("editing file with bad line ref"),
        )
        .add_step(
            |state: &State| {
                let output = state.outputs.get("edit-test.txt");
                assert!(output.is_some(), "expected edit-test.txt in outputs");
                let stdout = &output.unwrap().stdout;
                assert!(
                    stdout.contains("edit error:"),
                    "expected edit error in output, got: {stdout}"
                );
                assert!(
                    !stdout.contains("REPLACED"),
                    "should not contain 'REPLACED', got: {stdout}"
                );
            },
            AgentResponse::block("edit-test.txt", BlockMode::Close, "").with_task("closing"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn file_mode_shows_file() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("file-test.txt", BlockMode::Write, "file content here")
                .with_task("creating file"),
        )
        .add_step(
            |_state: &State| {},
            AgentResponse::block("file-test.txt", BlockMode::File, "").with_task("viewing file"),
        )
        .add_step(
            |state: &State| {
                let output = state.outputs.get("file-test.txt");
                assert!(output.is_some(), "expected file-test.txt in outputs");
                assert!(output.unwrap().stdout.contains("file content here"));
            },
            AgentResponse::block("file-test.txt", BlockMode::Close, "").with_task("closing"),
        );

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn task_persists_and_completes() {
    let agent = TestAgent::new()
        .add_step(
            |_state: &State| {},
            AgentResponse::block("build", BlockMode::Watch, "echo ok").with_task("check build"),
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

    TestRunner::new(agent).await.run().await;
}

#[tokio::test]
#[traced_test]
async fn mind_persists_across_ticks() {
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
            AgentResponse::block("build", BlockMode::Close, "").with_task("step 2: done"),
        );

    TestRunner::new(agent).await.run().await;
}
