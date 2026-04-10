use std::any::Any;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use async_openai::error::OpenAIError;
use serde::{Deserialize, Serialize};
use crate::prompt::Prompt;
use crate::types::{BlockMode, ParsedSegment};

mod test_agent;
mod llm;

pub use test_agent::TestAgent;
pub use llm::LlmAgent;

#[async_trait]
pub trait Agent: Send + Sync {
    async fn step(&self, prompt: &Prompt) -> Result<AgentResponse, AgentError>;

    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub reasoning: String,
    pub segments: Vec<ParsedSegment>,
}

impl AgentResponse {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            reasoning: String::new(),
            segments: Vec::new(),
        }
    }

    #[must_use]
    pub fn prose(text: impl Into<String>) -> Self {
        Self {
            reasoning: String::new(),
            segments: vec![ParsedSegment::Prose(text.into())],
        }
    }

    #[must_use]
    pub fn block(window: impl Into<String>, mode: BlockMode, content: impl Into<String>) -> Self {
        Self {
            reasoning: String::new(),
            segments: vec![ParsedSegment::Block {
                window: window.into(),
                mode,
                content: content.into(),
            }],
        }
    }

    #[must_use]
    pub fn and(mut self, other: AgentResponse) -> Self {
        self.segments.extend(other.segments);
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
    async fn step(&self, _prompt: &Prompt) -> Result<AgentResponse, AgentError> {
        Ok(AgentResponse::empty())
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
    async fn step(&self, _prompt: &Prompt) -> Result<AgentResponse, AgentError> {
        let idx = self.current.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(idx)
            .cloned()
            .ok_or_else(|| AgentError::Api("no more responses".into()))
    }
}

pub struct TestStep {
    check: Box<dyn Fn(&Prompt) + Send + Sync>,
    response: AgentResponse,
}
