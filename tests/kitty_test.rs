use std::time::Duration;
use tai::backend::kitty::KittyBackend;
use tai::backend::TerminalBackend;
use tai::types::LaunchOpts;

#[allow(clippy::expect_used)]
async fn spawn_backend() -> KittyBackend {
    KittyBackend::spawn(None, true)
        .await
        .expect("kitty should be available in PATH")
}

#[tokio::test]
async fn spawn_connects_to_kitty() {
    let backend = spawn_backend().await;
    assert!(backend.socket_path().exists());
}

#[tokio::test]
async fn list_windows_returns_initial() {
    let backend = spawn_backend().await;
    let windows = backend.list_windows().await.expect("list_windows should work");
    assert!(!windows.is_empty(), "kitty should have at least one initial window");
}

#[tokio::test]
async fn launch_creates_window() {
    let backend = spawn_backend().await;

    let opts = LaunchOpts::new(
        "test-echo".to_string(),
        vec!["bash".to_string(), "-c".to_string(), "echo hello-world".to_string()],
    );
    let window_id = backend.launch(&opts).await.expect("launch should succeed");

    let windows = backend.list_windows().await.expect("list_windows should work");
    let found = windows.iter().any(|w| w.id == window_id);
    assert!(found, "launched window should appear in list");
}

#[tokio::test]
async fn get_text_returns_content() {
    let backend = spawn_backend().await;

    let opts = LaunchOpts::new(
        "test-text".to_string(),
        vec!["bash".to_string(), "-c".to_string(), "echo marker-42".to_string()],
    );
    let window_id = backend.launch(&opts).await.expect("launch should succeed");

    tokio::time::sleep(Duration::from_secs(2)).await;

    let text = backend.get_text(&window_id).await.expect("get_text should work");
    assert!(
        text.contains("marker-42"),
        "get_text should contain output, got: {:?}",
        text
    );
}

#[tokio::test]
async fn send_text_to_window() {
    let backend = spawn_backend().await;

    let opts = LaunchOpts::new(
        "test-send".to_string(),
        vec!["cat".to_string()],
    );
    let window_id = backend.launch(&opts).await.expect("launch should succeed");
    tokio::time::sleep(Duration::from_millis(500)).await;

    backend
        .send_text(&window_id, "hello-from-tai\n")
        .await
        .expect("send_text should succeed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let text = backend.get_text(&window_id).await.expect("get_text should work");
    assert!(
        text.contains("hello-from-tai"),
        "sent text should appear in window, got: {:?}",
        text
    );
}

#[tokio::test]
async fn close_window() {
    let backend = spawn_backend().await;

    let opts = LaunchOpts::new(
        "test-close".to_string(),
        vec!["sleep".to_string(), "60".to_string()],
    );
    let window_id = backend.launch(&opts).await.expect("launch should succeed");

    let before = backend.list_windows().await.expect("list should work");
    let had_window = before.iter().any(|w| w.id == window_id);
    assert!(had_window);

    backend.close(&window_id).await.expect("close should succeed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let after = backend.list_windows().await.expect("list should work");
    let still_has = after.iter().any(|w| w.id == window_id);
    assert!(!still_has, "closed window should not appear in list");
}

#[tokio::test]
async fn set_title() {
    let backend = spawn_backend().await;

    let opts = LaunchOpts::new(
        "original-title".to_string(),
        vec!["sleep".to_string(), "60".to_string()],
    );
    let window_id = backend.launch(&opts).await.expect("launch should succeed");

    backend
        .set_title(&window_id, "new-title")
        .await
        .expect("set_title should succeed");

    let windows = backend.list_windows().await.expect("list should work");
    let found = windows
        .iter()
        .find(|w| w.id == window_id)
        .expect("window should exist");
    assert_eq!(found.title, "new-title");
}

#[tokio::test]
async fn process_watch_detects_exit() {
    use tai::backend::watch::ProcessWatch;

    let backend = spawn_backend().await;

    let opts = LaunchOpts::new(
        "test-watch".to_string(),
        vec!["bash".to_string(), "-c".to_string(), "echo quick-exit".to_string()],
    );
    let window_id = backend.launch(&opts).await.expect("launch should succeed");

    let mut watch: ProcessWatch<KittyBackend> =
        ProcessWatch::new(backend, Duration::from_millis(200));
    watch.track(window_id.clone());

    let mut events = Vec::new();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let result = watch.poll().await.expect("poll should work");
        events.extend(result);
        if !watch.is_tracked(&window_id) {
            break;
        }
    }

    assert!(!events.is_empty(), "should detect at_prompt for exited window");
    assert!(
        events[0].content.contains("quick-exit"),
        "captured content should contain output, got: {:?}",
        events[0].content
    );
}
