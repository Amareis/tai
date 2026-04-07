use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Trait для агента (L-модели).
///
/// Агент получает собранный промпт и возвращает текстовый ответ.
/// MVP: плоский промпт, без streaming, без thinking extraction.
#[async_trait]
pub trait Agent: Send + Sync {
    async fn step(&self, prompt: &str) -> Result<String, AgentError>;
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
    async fn step(&self, _prompt: &str) -> Result<String, AgentError> {
        Ok(String::new())
    }
}

/// Mock-агент для тестирования.
///
/// Возвращает заранее заданные ответы по очереди.
/// Каждый вызов `step()` возвращает следующий ответ из списка.
pub struct MockAgent {
    responses: Vec<String>,
    current: AtomicUsize,
}

impl MockAgent {
    #[must_use]
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            current: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn single(response: String) -> Self {
        Self::new(vec![response])
    }
}

#[async_trait]
impl Agent for MockAgent {
    async fn step(&self, _prompt: &str) -> Result<String, AgentError> {
        let idx = self.current.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(idx)
            .cloned()
            .ok_or_else(|| AgentError::Api("no more responses".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_agent_single() {
        let agent = MockAgent::single("hello".to_string());
        assert_eq!(agent.step("").await.unwrap(), "hello");
        assert!(agent.step("").await.is_err());
    }

    #[tokio::test]
    async fn test_mock_agent_sequence() {
        let agent = MockAgent::new(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(agent.step("").await.unwrap(), "a");
        assert_eq!(agent.step("").await.unwrap(), "b");
        assert_eq!(agent.step("").await.unwrap(), "c");
        assert!(agent.step("").await.is_err());
    }
}
