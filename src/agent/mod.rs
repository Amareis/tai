use std::any::Any;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use async_openai::error::OpenAIError;
use serde::{Deserialize, Serialize};
use crate::state::State;
use crate::types::{BlockMode, ParsedBlock};

mod test_agent;
mod llm;

pub use test_agent::TestAgent;
pub use llm::LlmAgent;

#[async_trait]
pub trait Agent: Send + Sync {
    async fn step(&self, state: &State) -> Result<AgentResponse, AgentError>;

    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentResponse {
    pub reasoning: String,
    pub segments: Vec<ParsedBlock>,
    pub task: String,
    #[serde(default)]
    pub complete: bool,
    #[serde(skip)]
    pub heredoc_violations: Vec<String>,
    #[serde(skip)]
    pub edit_parse_errors: Vec<String>,
}

impl AgentResponse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn block(window: impl Into<String>, mode: BlockMode, content: impl Into<String>) -> Self {
        let window = window.into();
        let content = content.into();
        let mode = if matches!(mode, BlockMode::Edit(_)) {
            match crate::response::edit_command::parse_edit_command(&content, None) {
                Ok(cmd) => BlockMode::Edit(Some(cmd)),
                Err(_) => BlockMode::Edit(None),
            }
        } else {
            mode
        };
        Self {
            reasoning: String::new(),
            segments: vec![ParsedBlock {
                window,
                mode,
                content,
                prose: None,
                dashboard: false,
            }],
            task: String::new(),
            complete: false,
            heredoc_violations: Vec::new(),
            edit_parse_errors: Vec::new(),
        }
    }

    #[must_use]
    pub fn and(mut self, other: AgentResponse) -> Self {
        self.segments.extend(other.segments);
        self
    }

    #[must_use]
    pub fn with_reasoning(mut self, reasoning: &str) -> Self {
        self.reasoning = reasoning.to_string();
        self
    }

    #[must_use]
    pub fn with_task(mut self, task: &str) -> Self {
        self.task = task.to_string();
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("API error: {0}")]
    Api(String),
    #[error("timeout")]
    Timeout,
    #[error("LLM error: {0}")]
    Llm(#[from] OpenAIError),
    #[error("response validation failed after retries: {0:?}")]
    InvalidResponse(Vec<String>),
}

pub struct NopAgent;

#[async_trait]
impl Agent for NopAgent {
    async fn step(&self, _state: &State) -> Result<AgentResponse, AgentError> {
        Ok(AgentResponse::new())
    }
}

pub struct MockAgent {
    responses: Vec<AgentResponse>,
    current: AtomicUsize,
}

impl MockAgent {
    #[must_use]
    pub fn new(responses: Vec<AgentResponse>) -> Self {
        Self {
            responses,
            current: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn single(response: AgentResponse) -> Self {
        Self::new(vec![response])
    }
}

#[async_trait]
impl Agent for MockAgent {
    async fn step(&self, _state: &State) -> Result<AgentResponse, AgentError> {
        let idx = self.current.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(idx)
            .cloned()
            .ok_or_else(|| AgentError::Api("no more responses".into()))
    }
}

pub struct TestStep {
    check: Box<dyn Fn(&State) + Send + Sync>,
    response: AgentResponse,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_response_new() {
        let resp = AgentResponse::new();
        assert!(resp.reasoning.is_empty());
        assert!(resp.segments.is_empty());
        assert!(resp.task.is_empty());
        assert!(!resp.complete);
        assert!(resp.heredoc_violations.is_empty());
        assert!(resp.edit_parse_errors.is_empty());
    }

    #[test]
    fn test_agent_response_block_watch() {
        let resp = AgentResponse::block("build", BlockMode::Watch, "cargo build");
        assert!(resp.reasoning.is_empty());
        assert_eq!(resp.segments.len(), 1);
        assert_eq!(resp.segments[0].window, "build");
        assert_eq!(resp.segments[0].mode, BlockMode::Watch);
        assert_eq!(resp.segments[0].content, "cargo build");
        assert!(resp.task.is_empty());
        assert!(!resp.complete);
    }

    #[test]
    fn test_agent_response_block_edit_parses_commands() {
        let resp = AgentResponse::block(
            "src/main.rs",
            BlockMode::Edit(None),
            "Exactly L1:old text\n<<'TAIDELIM'\nnew text\nTAIDELIM",
        );
        assert_eq!(resp.segments.len(), 1);
        if let BlockMode::Edit(ref cmd_opt) = resp.segments[0].mode {
            assert!(cmd_opt.is_some(), "edit command should be parsed from content");
        } else {
            panic!("expected Edit mode");
        }
    }

    #[test]
    fn test_agent_response_block_non_edit() {
        let resp = AgentResponse::block("test", BlockMode::Exec, "cargo test");
        assert_eq!(resp.segments[0].mode, BlockMode::Exec);
        assert_eq!(resp.segments[0].content, "cargo test");
    }

    #[test]
    fn test_agent_response_and() {
        let a = AgentResponse::block("build", BlockMode::Watch, "cargo build");
        let b = AgentResponse::block("test", BlockMode::Exec, "cargo test");
        let combined = a.and(b);
        assert_eq!(combined.segments.len(), 2);
        assert_eq!(combined.segments[0].window, "build");
        assert_eq!(combined.segments[1].window, "test");
    }

    #[test]
    fn test_agent_response_with_reasoning() {
        let resp = AgentResponse::new().with_reasoning("I need to fix this");
        assert_eq!(resp.reasoning, "I need to fix this");
    }

    #[test]
    fn test_agent_response_with_task() {
        let resp = AgentResponse::new().with_task("Refactor the module");
        assert_eq!(resp.task, "Refactor the module");
    }

    #[test]
    fn test_agent_response_chained_builders() {
        let resp = AgentResponse::block("build", BlockMode::Watch, "cargo build")
            .with_reasoning("checking build")
            .with_task("Fix build errors");
        assert_eq!(resp.reasoning, "checking build");
        assert_eq!(resp.task, "Fix build errors");
        assert_eq!(resp.segments.len(), 1);
    }

    #[test]
    fn test_mock_agent_returns_responses_in_order() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let a = AgentResponse::block("a", BlockMode::Watch, "cmd a");
        let b = AgentResponse::block("b", BlockMode::Exec, "cmd b");
        let agent = MockAgent::new(vec![a.clone(), b.clone()]);
        let state = State::default();
        let r1 = rt.block_on(agent.step(&state)).unwrap();
        let r2 = rt.block_on(agent.step(&state)).unwrap();
        assert_eq!(r1.segments[0].window, "a");
        assert_eq!(r2.segments[0].window, "b");
    }

    #[test]
    fn test_mock_agent_exhausted_returns_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let agent = MockAgent::new(vec![]);
        let state = State::default();
        let result = rt.block_on(agent.step(&state));
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_agent_single() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = AgentResponse::block("x", BlockMode::Watch, "cmd x");
        let agent = MockAgent::single(resp.clone());
        let state = State::default();
        let result = rt.block_on(agent.step(&state)).unwrap();
        assert_eq!(result.segments[0].window, "x");
    }

    #[test]
    fn test_nop_agent() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let agent = NopAgent;
        let state = State::default();
        let result = rt.block_on(agent.step(&state)).unwrap();
        assert!(result.segments.is_empty());
        assert!(result.task.is_empty());
    }
}
