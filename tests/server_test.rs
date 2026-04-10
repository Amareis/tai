#![cfg(test)]

use assert_matches::assert_matches;
use std::time::Duration;
use tai::agent::{AgentResponse, TestAgent};
use tai::types::BlockMode;
use tai::backend::Terminal;
use tai::core::Server;
use tai::create_server;
use tai::prompt::Prompt;
use tracing_test::traced_test;

fn assert_test_agent(server: &Server) {
    server.agent.as_any().expect("expected as_any in TestAgent")
        .downcast_ref::<TestAgent>()
        .expect("expected TestAgent").assert_all_consumed();
}

#[tokio::test]
#[traced_test]
async fn tick_empty_session() {
    let agent = TestAgent::new().add_step(
        |prompt: &Prompt| {
            assert_eq!(prompt.dashboard.len(), 1);
            assert_eq!(prompt.focused_windows.len(), 1);
        },
        AgentResponse::empty(),
    );

    let mut server = create_server(None, Box::new(agent), true).await.unwrap();
    server.tick().await.unwrap();

    assert_test_agent(&server);
}

#[tokio::test]
#[traced_test]
async fn tick_agent_launches_window_and_session_tracks_it() {
    let agent = TestAgent::new()
        .add_step(
            |prompt: &Prompt| {
                assert_eq!(prompt.dashboard.len(), 1);
            },
            AgentResponse::block(
                "tai",
                BlockMode::Text,
                "launch --title hello -- echo marker-xyz",
            ),
        )
        .add_step(
            |prompt: &Prompt| {
                let w = prompt
                    .dashboard
                    .first()
                    .expect("session should track launched window");
                assert_eq!(w.title, "nc");
                assert_matches!(
                    w,
                    Terminal {
                        last_cmd_exit_status: Some(0),
                        ..
                    },
                    "Window should be closed"
                );
            },
            AgentResponse::empty(),
        );

    let mut server = create_server(None, Box::new(agent), true).await.unwrap();
    server.tick().await.unwrap();
    server
        .wait_trigger(Some(Duration::from_secs(1)))
        .await
        .unwrap();
    server.tick().await.unwrap();

    assert_test_agent(&server);
}