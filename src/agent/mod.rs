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
    pub mind: String,
    #[serde(skip)]
    pub heredoc_violations: Vec<String>,
}

impl AgentResponse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn block(window: impl Into<String>, mode: BlockMode, content: impl Into<String>) -> Self {
        Self {
            reasoning: String::new(),
            segments: vec![ParsedBlock {
                window: window.into(),
                mode,
                content: content.into(),
                prose: None,
                dashboard: false,
            }],
            mind: String::new(),
            heredoc_violations: Vec::new(),
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
    pub fn with_mind(mut self, mind: &str) -> Self {
        self.mind = mind.to_string();
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
