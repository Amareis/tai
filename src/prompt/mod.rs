use crate::agent::AgentResponse;

const SYSTEM_PROMPT: &str = include_str!("system_prompt.txt");

#[derive(Debug, Clone)]
pub struct TrackedView {
    pub title: String,
    pub output: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub system: String,
    pub tracked: Vec<TrackedView>,
    pub previous_response: Option<AgentResponse>,
}

impl Prompt {
    #[must_use]
    pub fn build(tracked: Vec<TrackedView>, previous_response: Option<AgentResponse>) -> Self {
        Self {
            system: SYSTEM_PROMPT.to_string(),
            tracked,
            previous_response,
        }
    }
}
