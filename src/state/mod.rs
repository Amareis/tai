use crate::agent::AgentResponse;
use crate::backend::CmdOutput;
use crate::types::ParsedBlock;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct State {
    pub system: String,
    pub segments: Vec<ParsedBlock>,
    pub outputs: HashMap<String, CmdOutput>,
    pub tick_n: u64,
    pub mind: String,
    pub response: AgentResponse,
}

impl State {
    #[must_use]
    pub fn build(
        system: String,
        segments: Vec<ParsedBlock>,
        outputs: HashMap<String, CmdOutput>,
        tick_n: u64,
        mind: String,
        response: AgentResponse,
    ) -> Self {
        Self {
            system,
            segments,
            outputs,
            tick_n,
            mind,
            response,
        }
    }
}
