use crate::agent::{Agent, AgentError, AgentResponse, TestStep};
use crate::state::State;
use async_trait::async_trait;
use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct TestAgent {
    steps: Vec<TestStep>,
    current: AtomicUsize,
}

impl Default for TestAgent {
    fn default() -> Self {
        Self::new()
    }
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
    pub fn add_step(
        mut self,
        check: impl Fn(&State) + Send + Sync + 'static,
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
    async fn step(&self, state: &State) -> Result<AgentResponse, AgentError> {
        let idx = self.current.load(Ordering::SeqCst);
        let step = self
            .steps
            .get(idx)
            .ok_or_else(|| AgentError::Api(format!("no test step at index {idx}")))?;

        self.current.fetch_add(1, Ordering::SeqCst);
        (step.check)(state);

        Ok(step.response.clone())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
