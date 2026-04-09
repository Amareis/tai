use std::path::PathBuf;
use std::time::Duration;
use tai::backend::kitty::KittyBackend;
use tai::backend::{BackendCmd, CmdResponse, LaunchCmd, Terminal, TerminalBackend};

#[allow(clippy::expect_used)]
async fn spawn_backend_with_path() -> (KittyBackend, PathBuf) {
    let socket_path =
        std::env::temp_dir().join(format!("test-kitty-{}.sock", uuid::Uuid::new_v4()));

    (
        KittyBackend::spawn(&[], &socket_path, true)
            .await
            .expect("kitty should be available in PATH"),
        socket_path,
    )
}

async fn spawn_backend() -> KittyBackend {
    spawn_backend_with_path().await.0
}

#[tokio::test]
async fn spawn_connects_to_kitty() {
    let (_back, socket_path) = spawn_backend_with_path().await;
    assert!(socket_path.exists());
}

#[tokio::test]
async fn list_windows_returns_initial() {
    let backend = spawn_backend().await;
    let CmdResponse::Windows(windows) = backend
        .execute(BackendCmd::List)
        .await
        .expect("list should work") else { panic!("expected Windows response") };
    assert!(
        !windows.is_empty(),
        "kitty should have at least one initial window"
    );
}

#[tokio::test]
async fn launch_creates_window() {
    let backend = spawn_backend().await;

    let CmdResponse::WindowCreated(window_id) = backend
        .execute(BackendCmd::Launch(LaunchCmd {
            title: Some("test-echo".to_string()),
            command: vec![
                "bash".to_string(),
                "-c".to_string(),
                "echo hello-world".to_string(),
            ],
        }))
        .await
        .expect("launch should succeed") else { panic!("expected WindowCreated response") };

    let CmdResponse::Windows(windows) = backend
        .execute(BackendCmd::List)
        .await
        .expect("list should work") else { panic!("expected Windows response") };
    let found = windows.iter().any(|w| w.id == window_id);
    assert!(found, "launched window should appear in list");
}

#[tokio::test]
async fn get_text_returns_content() {
    let backend = spawn_backend().await;

    let CmdResponse::WindowCreated(window_id) = backend
        .execute(BackendCmd::Launch(LaunchCmd {
            title: Some("test-text".to_string()),
            command: vec![
                "echo marker-42".to_string(),
            ],
        }))
        .await
        .expect("launch should succeed") else { panic!("expected WindowCreated") };

    tokio::time::sleep(Duration::from_secs(2)).await;

    let CmdResponse::Text(text) = backend
        .execute(BackendCmd::Get(tai::backend::GetTextCmd {
            window_id: window_id.clone(),
        }))
        .await
        .expect("get_text should work") else { panic!("expected Text response") };
    assert!(
        text.contains("marker-42"),
        "get_text should contain output, got: {text:?}",
    );
}

#[tokio::test]
async fn send_text_to_window() {
    let backend = spawn_backend().await;

    let CmdResponse::WindowCreated(window_id) = backend
        .execute(BackendCmd::Launch(LaunchCmd {
            title: Some("test-send".to_string()),
            command: vec!["cat".to_string()],
        }))
        .await
        .expect("launch should succeed") else { panic!("expected WindowCreated") };
    tokio::time::sleep(Duration::from_millis(500)).await;

    backend
        .execute(BackendCmd::Send(tai::backend::SendTextCmd {
            window: window_id.clone(),
            text: vec!["hello-from-tai\n".to_string()],
        }))
        .await
        .expect("send_text should succeed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let CmdResponse::Text(text) = backend
        .execute(BackendCmd::Get(tai::backend::GetTextCmd {
            window_id: window_id.clone(),
        }))
        .await
        .expect("get_text should work") else { panic!("expected Text") };
    assert!(
        text.contains("hello-from-tai"),
        "sent text should appear in window, got: {text:?}",
    );
}

#[tokio::test]
async fn close_window() {
    let backend = spawn_backend().await;

    let CmdResponse::WindowCreated(window_id) = backend
        .execute(BackendCmd::Launch(LaunchCmd {
            title: Some("test-close".to_string()),
            command: vec!["sleep".to_string(), "60".to_string()],
        }))
        .await
        .expect("launch should succeed") else { panic!("expected WindowCreated") };

    let CmdResponse::Windows(before) = backend
        .execute(BackendCmd::List)
        .await
        .expect("list should work") else { panic!("expected Windows") };
    let had_window = before.iter().any(|w| w.id == window_id);
    assert!(had_window);

    backend
        .execute(BackendCmd::Close(tai::backend::CloseCmd {
            window_id: window_id.clone(),
        }))
        .await
        .expect("close should succeed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let CmdResponse::Windows(after) = backend
        .execute(BackendCmd::List)
        .await
        .expect("list should work") else { panic!("expected Windows") };
    let still_has = after.iter().any(|w| w.id == window_id);
    assert!(!still_has, "closed window should not appear in list");
}

#[tokio::test]
async fn set_title() {
    let backend = spawn_backend().await;

    let CmdResponse::WindowCreated(window_id) = backend
        .execute(BackendCmd::Launch(LaunchCmd {
            title: Some("original-title".to_string()),
            command: vec!["sleep".to_string(), "60".to_string()],
        }))
        .await
        .expect("launch should succeed") else { panic!("expected WindowCreated") };

    backend
        .execute(BackendCmd::Title(tai::backend::SetTitleCmd {
            window_id: window_id.clone(),
            title: vec!["new-title".to_string()],
        }))
        .await
        .expect("set_title should succeed");

    let CmdResponse::Windows(windows) = backend
        .execute(BackendCmd::List)
        .await
        .expect("list should work") else { panic!("expected Windows") };
    let found = windows
        .iter()
        .find(|w| w.id == window_id)
        .expect("window should exist");
    assert_eq!(found.title, "new-title");
}

#[tokio::test]
async fn watcher_detects_exit() {
    use tai::backend::watch::Watcher;

    let backend = spawn_backend().await;

    let CmdResponse::WindowCreated(window_id) = backend
        .execute(BackendCmd::Launch(LaunchCmd {
            title: Some("test-watch".to_string()),
            command: vec![
                "bash".to_string(),
                "-c".to_string(),
                "echo quick-exit".to_string(),
            ],
        }))
        .await
        .expect("launch should succeed") else { panic!("expected WindowCreated") };

    let mut watcher = Watcher::new();
    watcher.track(window_id.clone(), true);

    let mut detected = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let exited = watcher.poll_exited(&backend).await.expect("poll should work");
        if let Some(Terminal {id, ..}) = exited.first() && *id == window_id {
            detected = true;
        }
        if detected {
            break;
        }
    }

    assert!(detected, "should detect window exit");
}
