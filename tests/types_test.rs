use chrono::Utc;
use std::path::PathBuf;

use tai::config::{self, Config};
use tai::types::{
    BlockMode, LaunchOpts, ParsedSegment, Session, TaiCommand, TickTrigger, Window, WindowState,
};

#[test]
fn window_state_active_roundtrip() {
    let state = WindowState::Active {
        backend_id: "kitty-0".to_string(),
        pid: 12345,
        title: "bash".to_string(),
    };
    let json = serde_json::to_string(&state).expect("serialize");
    let de: WindowState = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(state, de);
}

#[test]
fn window_state_frozen_roundtrip() {
    let state = WindowState::Frozen {
        content: "hello\nworld".to_string(),
        exit_code: 0,
        captured_at: Utc::now(),
    };
    let json = serde_json::to_string(&state).expect("serialize");
    let de: WindowState = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(state, de);
}

#[test]
fn window_state_archived_roundtrip() {
    let state = WindowState::Archived {
        file_path: PathBuf::from("sessions/default/history/build-1.txt"),
        exit_code: 1,
    };
    let json = serde_json::to_string(&state).expect("serialize");
    let de: WindowState = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(state, de);
}

#[test]
fn window_state_helpers() {
    let active = WindowState::Active {
        backend_id: "1".to_string(),
        pid: 1,
        title: "t".to_string(),
    };
    assert!(active.is_active());
    assert!(!active.is_frozen());
    assert_eq!(active.exit_code(), None);

    let frozen = WindowState::Frozen {
        content: String::new(),
        exit_code: 42,
        captured_at: Utc::now(),
    };
    assert!(!frozen.is_active());
    assert!(frozen.is_frozen());
    assert_eq!(frozen.exit_code(), Some(42));
}

#[test]
fn window_roundtrip() {
    let window = Window {
        id: "abc-123".to_string(),
        state: WindowState::Active {
            backend_id: "kitty-0".to_string(),
            pid: 999,
            title: "build".to_string(),
        },
        focused: true,
        summary: Some("building project".to_string()),
        tags: vec!["ci".to_string(), "rust".to_string()],
    };
    let json = serde_json::to_string(&window).expect("serialize");
    let de: Window = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(window, de);
}

#[test]
fn window_new_active() {
    let w = Window::new_active("kitty-1".to_string(), 42, "test".to_string());
    assert!(w.state.is_active());
    assert!(!w.focused);
    assert!(w.summary.is_none());
    assert!(w.tags.is_empty());
    assert!(!w.id.is_empty());
}

#[test]
fn session_roundtrip() {
    let session = Session {
        id: "sess-1".to_string(),
        windows: vec![
            Window::new_active("k-0".to_string(), 1, "bash".to_string()),
            Window {
                id: "w-2".to_string(),
                state: WindowState::Frozen {
                    content: "done".to_string(),
                    exit_code: 0,
                    captured_at: Utc::now(),
                },
                focused: false,
                summary: Some("completed".to_string()),
                tags: vec![],
            },
        ],
        mind_path: PathBuf::from("sessions/default/mind.md"),
    };
    let json = serde_json::to_string(&session).expect("serialize");
    let de: Session = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(session, de);
}

#[test]
fn session_queries() {
    let mut session = Session::new("s-1".to_string(), PathBuf::from("mind.md"));

    let w1 = Window::new_active("k-0".to_string(), 1, "bash".to_string());
    let w1_id = w1.id.clone();
    session.windows.push(w1);

    let mut w2 = Window::new_active("k-1".to_string(), 2, "build".to_string());
    w2.focused = true;
    let w2_id = w2.id.clone();
    session.windows.push(w2);

    assert_eq!(session.active_windows().count(), 2);
    assert_eq!(session.focused_windows().count(), 1);

    let found = session.find_window(&w1_id);
    assert!(found.is_some());
    assert_eq!(found.map(|w| w.id.as_str()), Some(w1_id.as_str()));

    let found_mut = session.find_window_mut(&w2_id);
    assert!(found_mut.is_some());
}

#[test]
fn launch_opts_roundtrip() {
    let opts = LaunchOpts::new("tests".to_string(), "cargo test".to_string());
    let json = serde_json::to_string(&opts).expect("serialize");
    let de: LaunchOpts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(opts, de);
    assert!(opts.hold);
    assert!(opts.shell.is_none());
}

#[test]
fn launch_opts_with_shell() {
    let opts = LaunchOpts {
        title: "repl".to_string(),
        command: "python3".to_string(),
        shell: Some("bash".to_string()),
        hold: true,
    };
    let json = serde_json::to_string(&opts).expect("serialize");
    let de: LaunchOpts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(opts, de);
}

#[test]
fn tai_command_roundtrip() {
    let commands = vec![
        TaiCommand::Launch {
            title: "build".to_string(),
            cmd: "cargo build".to_string(),
            shell: None,
        },
        TaiCommand::Close {
            window_id: "w-1".to_string(),
        },
        TaiCommand::Focus {
            window_id: "w-2".to_string(),
        },
        TaiCommand::Summarize {
            window_id: "w-3".to_string(),
        },
    ];

    for cmd in commands {
        let json = serde_json::to_string(&cmd).expect("serialize");
        let de: TaiCommand = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cmd, de);
    }
}

#[test]
fn block_mode_roundtrip() {
    let modes = vec![BlockMode::Text, BlockMode::Keys];
    for mode in modes {
        let json = serde_json::to_string(&mode).expect("serialize");
        let de: BlockMode = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(mode, de);
    }
}

#[test]
fn parsed_segment_equality() {
    let prose = ParsedSegment::Prose("hello".to_string());
    let block = ParsedSegment::Block {
        target: "bash".to_string(),
        mode: BlockMode::Text,
        content: "ls".to_string(),
    };
    let cmd = ParsedSegment::TaiCommand(TaiCommand::Close {
        window_id: "w-1".to_string(),
    });
    let invalid = ParsedSegment::Invalid {
        raw_header: "???".to_string(),
        error: "bad header".to_string(),
    };

    assert_eq!(prose, ParsedSegment::Prose("hello".to_string()));
    assert_eq!(
        block,
        ParsedSegment::Block {
            target: "bash".to_string(),
            mode: BlockMode::Text,
            content: "ls".to_string()
        }
    );
    assert_eq!(prose, prose.clone());
    assert_eq!(block, block.clone());
    assert_eq!(cmd, cmd.clone());
    assert_eq!(invalid, invalid.clone());
}

#[test]
fn tick_trigger_equality() {
    let triggers = vec![
        TickTrigger::WindowExited {
            window_id: "w-1".to_string(),
            exit_code: 0,
        },
        TickTrigger::UserMessage,
        TickTrigger::IdleTimeout,
    ];
    for trigger in &triggers {
        assert_eq!(trigger, &trigger.clone());
    }
}

#[test]
fn config_default_roundtrip() {
    let config = Config::default();
    let toml_str = toml::to_string(&config).expect("serialize to toml");
    let de: Config = toml::from_str(&toml_str).expect("deserialize from toml");
    assert_eq!(config, de);
}

#[test]
fn config_load_from_file() {
    let config_content = r#"
[kernel]
poll_interval_ms = 250
idle_timeout_secs = 30

[model]
model_name = "gpt-4"
provider = "openai"
max_tokens = 64000

[session]
dir = "/tmp/tai-sessions"

[backend]
backend_type = "tmux"
"#;
    let tmp = tempfile::NamedTempFile::new().expect("create temp file");
    std::fs::write(tmp.path(), config_content).expect("write config");
    let config = config::load(tmp.path()).expect("load config");

    assert_eq!(config.kernel.poll_interval_ms, 250);
    assert_eq!(config.kernel.idle_timeout_secs, 30);
    assert_eq!(config.model.model_name, "gpt-4");
    assert_eq!(config.model.provider, "openai");
    assert_eq!(config.model.max_tokens, 64_000);
    assert_eq!(config.session.dir, PathBuf::from("/tmp/tai-sessions"));
    assert_eq!(config.backend.backend_type, "tmux");
}

#[test]
fn config_load_missing_file_returns_default() {
    let config = config::load_or_default(PathBuf::from("/nonexistent/config.toml").as_path());
    assert_eq!(config, Config::default());
}

#[test]
fn config_actual_file() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let config_path = PathBuf::from(manifest_dir).join("config.toml");
    if config_path.exists() {
        let config = config::load(&config_path).expect("load config.toml");
        assert_eq!(config.kernel.poll_interval_ms, 500);
        assert_eq!(config.backend.backend_type, "kitty");
    }
}
