use crate::backend::CmdOutput;
use crate::types::ParsedBlock;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct State {
    pub system: String,
    pub instructions: String,
    pub segments: Vec<ParsedBlock>,
    pub outputs: HashMap<String, CmdOutput>,
    pub tick_n: u64,
    pub task: String,
    pub is_completed: bool,
}

impl State {
    #[must_use]
    pub fn build(
        system: String,
        instructions: String,
        segments: Vec<ParsedBlock>,
        outputs: HashMap<String, CmdOutput>,
        tick_n: u64,
    ) -> Self {
        Self {
            system,
            instructions,
            segments,
            outputs,
            tick_n,
            task: String::new(),
            is_completed: false,
        }
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.is_completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::CmdOutput;
    use crate::types::{BlockMode, ParsedBlock};
    use std::collections::HashMap;

    #[test]
    fn test_build_default_values() {
        let state = State::build(
            "system".to_string(),
            "instructions".to_string(),
            Vec::new(),
            HashMap::new(),
            0,
        );
        assert_eq!(state.system, "system");
        assert_eq!(state.instructions, "instructions");
        assert!(state.segments.is_empty());
        assert!(state.outputs.is_empty());
        assert_eq!(state.tick_n, 0);
        assert!(state.task.is_empty());
        assert!(!state.is_completed);
    }

    #[test]
    fn test_build_with_values() {
        let segments = vec![ParsedBlock {
            window: "build".to_string(),
            mode: BlockMode::Watch,
            content: "cargo build".to_string(),
            prose: None,
            dashboard: false,
        }];
        let mut outputs = HashMap::new();
        outputs.insert(
            "build".to_string(),
            CmdOutput {
                exit_code: 0,
                stdout: "ok".to_string(),
            },
        );
        let state = State::build(
            "sys".to_string(),
            "instr".to_string(),
            segments.clone(),
            outputs.clone(),
            5,
        );
        assert_eq!(state.segments, segments);
        assert_eq!(state.outputs, outputs);
        assert_eq!(state.tick_n, 5);
        assert!(state.task.is_empty());
        assert!(!state.is_completed);
    }

    #[test]
    fn test_is_complete_false() {
        let state = State::default();
        assert!(!state.is_complete());
    }

    #[test]
    fn test_is_complete_true() {
        let mut state = State::default();
        state.is_completed = true;
        assert!(state.is_complete());
    }

    #[test]
    fn test_default() {
        let state = State::default();
        assert!(state.system.is_empty());
        assert!(state.instructions.is_empty());
        assert!(state.segments.is_empty());
        assert!(state.outputs.is_empty());
        assert_eq!(state.tick_n, 0);
        assert!(state.task.is_empty());
        assert!(!state.is_completed);
    }
}
