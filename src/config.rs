use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Конфигурация TAI, загружаемая из `config.toml`.
///
/// Разделы конфигурации соответствуют основным подсистемам:
/// - `kernel` — параметры главного цикла (tick triggers, timeouts)
/// - `model` — параметры L-модели (API key, модель, лимиты)
/// - `session` — пути к данным сессий
/// - `backend` — параметры терминального backend'а
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Config {
    #[serde(default)]
    pub kernel: KernelConfig,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub backend: BackendConfig,
}

/// Параметры главного цикла ядра.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KernelConfig {
    /// Интервал опроса окон (мс). По умолчанию 500мс.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    /// Idle timeout (сек). Если нет событий — статусный тик. По умолчанию 60с.
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    /// Timeout выполнения одного блока (сек). По умолчанию 30с.
    #[serde(default = "default_block_timeout_secs")]
    pub block_timeout_secs: u64,
}

fn default_poll_interval_ms() -> u64 {
    500
}
fn default_idle_timeout_secs() -> u64 {
    60
}
fn default_block_timeout_secs() -> u64 {
    30
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: default_poll_interval_ms(),
            idle_timeout_secs: default_idle_timeout_secs(),
            block_timeout_secs: default_block_timeout_secs(),
        }
    }
}

/// Параметры L-модели (Claude, GPT и т.д.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelConfig {
    /// API ключ. Если не указан — берётся из переменной окружения.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Имя модели (например, "claude-sonnet-4-20250514")
    #[serde(default = "default_model_name")]
    pub model_name: String,
    /// Максимальное количество токенов в контексте
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Провайдер LLM (anthropic, openai, etc.)
    #[serde(default = "default_provider")]
    pub provider: String,
}

fn default_model_name() -> String {
    "claude-sonnet-4-20250514".to_string()
}
fn default_max_tokens() -> usize {
    128_000
}
fn default_provider() -> String {
    "anthropic".to_string()
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model_name: default_model_name(),
            max_tokens: default_max_tokens(),
            provider: default_provider(),
        }
    }
}

/// Параметры хранилища сессий.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionConfig {
    /// Директория для данных сессий (по умолчанию `./sessions`)
    #[serde(default = "default_sessions_dir")]
    pub dir: PathBuf,
    /// Имя сессии по умолчанию
    #[serde(default = "default_session_name")]
    pub default_name: String,
}

fn default_sessions_dir() -> PathBuf {
    PathBuf::from("sessions")
}
fn default_session_name() -> String {
    "default".to_string()
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            dir: default_sessions_dir(),
            default_name: default_session_name(),
        }
    }
}

/// Параметры терминального backend'а.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackendConfig {
    /// Тип backend'а: "kitty" (по умолчанию), "tmux" (future)
    #[serde(default = "default_backend_type")]
    pub backend_type: String,
}

fn default_backend_type() -> String {
    "kitty".to_string()
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend_type: default_backend_type(),
        }
    }
}

/// Ошибки загрузки конфигурации.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Read(#[source] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[source] toml::de::Error),
}

/// Загрузить конфигурацию из файла. Если файл не найден — вернуть defaults.
pub fn load(path: &std::path::Path) -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
    toml::from_str(&content).map_err(ConfigError::Parse)
}

/// Загрузить конфигурацию, с fallback на defaults при отсутствии файла.
pub fn load_or_default(path: &std::path::Path) -> Config {
    load(path).unwrap_or_default()
}
