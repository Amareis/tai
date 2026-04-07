use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::prompt::Prompt;
use crate::types::{BlockMode, ParsedSegment};

#[async_trait]
pub trait Agent: Send + Sync {
    async fn step(&self, prompt: &Prompt) -> Result<AgentResponse, AgentError>;
}

#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub segments: Vec<ParsedSegment>,
}

impl AgentResponse {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    #[must_use]
    pub fn prose(text: impl Into<String>) -> Self {
        Self {
            segments: vec![ParsedSegment::Prose(text.into())],
        }
    }

    #[must_use]
    pub fn block(window: impl Into<String>, mode: BlockMode, content: impl Into<String>) -> Self {
        Self {
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

pub struct TestAgent {
    steps: Vec<TestStep>,
    current: AtomicUsize,
}

impl TestAgent {
    #[must_use]
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            current: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn step(
        mut self,
        check: impl Fn(&Prompt) + Send + Sync + 'static,
        response: AgentResponse,
    ) -> Self {
        self.steps.push(TestStep {
            check: Box::new(check),
            response,
        });
        self
    }

    /// # Panics
    /// If not all test steps were consumed.
    pub fn assert_all_consumed(&self) {
        let current = self.current.load(Ordering::SeqCst);
        assert_eq!(
            current,
            self.steps.len(),
            "not all test steps consumed: {}/{}",
            current,
            self.steps.len()
        );
    }
}

#[async_trait]
impl Agent for TestAgent {
    async fn step(&self, prompt: &Prompt) -> Result<AgentResponse, AgentError> {
        let idx = self.current.fetch_add(1, Ordering::SeqCst);
        let step = self
            .steps
            .get(idx)
            .ok_or_else(|| AgentError::Api(format!("no test step at index {idx}")))?;

        (step.check)(prompt);

        Ok(step.response.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_agent_single() {
        let agent = MockAgent::single(AgentResponse::prose("hello"));
        let prompt = Prompt {
            system: String::new(),
            dashboard: vec![],
            focused_windows: vec![],
            previous_response: None,
            status: crate::prompt::StatusInfo {
                active: 0,
                frozen: 0,
                focused: 0,
            },
        };
        let resp = agent.step(&prompt).await.unwrap();
        assert_eq!(resp.segments.len(), 1);
        assert!(agent.step(&prompt).await.is_err());
    }

    #[tokio::test]
    async fn test_mock_agent_sequence() {
        let agent = MockAgent::new(vec![
            AgentResponse::prose("a"),
            AgentResponse::prose("b"),
        ]);
        let prompt = Prompt {
            system: String::new(),
            dashboard: vec![],
            focused_windows: vec![],
            previous_response: None,
            status: crate::prompt::StatusInfo {
                active: 0,
                frozen: 0,
                focused: 0,
            },
        };
        assert_eq!(agent.step(&prompt).await.unwrap().segments.len(), 1);
        assert_eq!(agent.step(&prompt).await.unwrap().segments.len(), 1);
        assert!(agent.step(&prompt).await.is_err());
    }

    #[tokio::test]
    async fn test_agent_response_builder() {
        let r = AgentResponse::empty();
        assert!(r.segments.is_empty());

        let r = AgentResponse::prose("hello");
        assert_eq!(r.segments.len(), 1);

        let r = AgentResponse::block("win", BlockMode::Cmd, "launch -- bash");
        assert_eq!(r.segments.len(), 1);

        let r = AgentResponse::prose("text").and(AgentResponse::block("w", BlockMode::Text, "ls"));
        assert_eq!(r.segments.len(), 2);
    }
}
