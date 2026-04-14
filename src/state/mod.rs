use crate::backend::CmdOutput;
use crate::types::ParsedBlock;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct State {
    pub system: String,
    pub segments: Vec<ParsedBlock>,
    pub outputs: HashMap<String, CmdOutput>,
    pub tick_n: u64,
}

impl State {
    #[must_use]
    pub fn build(
        system: String,
        segments: Vec<ParsedBlock>,
        outputs: HashMap<String, CmdOutput>,
        tick_n: u64,
    ) -> Self {
        Self {
            system,
            segments,
            outputs,
            tick_n,
        }
    }
}
