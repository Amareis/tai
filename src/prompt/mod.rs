use crate::agent::AgentResponse;

const SYSTEM_PROMPT: &str = include_str!("system_prompt.txt");

#[derive(Debug, Clone)]
pub struct TrackedView {
    pub title: String,
    pub output: String,
    pub exit_code: i32,
    pub rerun: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub system: String,
    pub tracked: Vec<TrackedView>,
    pub previous_response: Option<AgentResponse>,
    pub tick_n: u64,
}

impl Prompt {
    #[must_use]
    pub fn build(
        tracked: Vec<TrackedView>,
        previous_response: Option<AgentResponse>,
        tick_n: u64,
    ) -> Self {
        Self {
            system: SYSTEM_PROMPT.to_string(),
            tracked,
            previous_response,
            tick_n,
        }
    }
}
