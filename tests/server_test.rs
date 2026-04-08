#![cfg(test)]

use assert_matches::assert_matches;
use tai::backend::kitty::KittyBackend;
use tai::core::utils::bind;
use tai::core::Server;
use tai::agent::{TestAgent, AgentResponse};
use tai::prompt::{Prompt, WindowStateKind};
use tai::types::{BlockMode, Session};

fn empty_session() -> Session {
    Session::new("test".to_string(), std::env::temp_dir().join("tai-test-mind.md"))
}

async fn spawn_server(agent: TestAgent, session: Session) -> (Server, tokio::task::JoinHandle<()>) {
    let kitty_socket =
        std::env::temp_dir().join(format!("test-kitty-{}.sock", uuid::Uuid::new_v4()));
    let client_socket =
        std::env::temp_dir().join(format!("test-server-{}.sock", uuid::Uuid::new_v4()));

    let backend = KittyBackend::spawn(&[], &kitty_socket, true)
        .await
        .expect("kitty should be in PATH");

    let (listener, _guard) = bind(&client_socket).await.unwrap();

    let sp = client_socket.clone();
    let conn_guard = tokio::spawn(async move {
        let _stream = tokio::net::UnixStream::connect(&sp).await.unwrap();
        std::future::pending::<()>().await;
    });

    let server = Server::accept(
        &listener,
        Box::new(backend),
        session,
        Box::new(agent),
    )
    .await
    .unwrap();

    (server, conn_guard)
}

#[tokio::test]
async fn tick_empty_session() {
    let agent = TestAgent::new().step(
        |prompt: &Prompt| {
            assert!(prompt.dashboard.is_empty());
            assert!(prompt.focused_windows.is_empty());
        },
        AgentResponse::empty(),
    );

    let (mut server, _guard) = spawn_server(agent, empty_session()).await;
    server.tick().await.unwrap();
}

#[tokio::test]
async fn tick_agent_launches_window_and_session_tracks_it() {
    let agent = TestAgent::new()
        .step(
            |prompt: &Prompt| {
                assert!(prompt.dashboard.is_empty());
            },
            AgentResponse::block("tai", BlockMode::Cmd, "launch --title hello -- echo marker-xyz"),
        )
        .step(
            |prompt: &Prompt| {
                let w = prompt.dashboard.first().expect("session should track launched window");
                assert_eq!(w.title, "hello");
                assert_matches!(w.state_kind, WindowStateKind::Frozen {exit_code: 0}, "Window should be closed");

            },
            AgentResponse::empty(),
        );

    let (mut server, _guard) = spawn_server(agent, empty_session()).await;
    server.tick().await.unwrap();
    server.tick().await.unwrap();
}
