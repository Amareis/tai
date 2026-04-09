use std::path::PathBuf;
use tai::config::{self, Config};

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
