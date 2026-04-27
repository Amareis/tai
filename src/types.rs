use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockMode {
    Watch,
    Close,
    Exec,
    Ask,
    File,
    Edit(Option<crate::response::edit_command::EditCommand>),
    Write,
    Task,
    Delegate,
}

impl std::str::FromStr for BlockMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "watch" | "view" => Ok(Self::Watch),
            "close" => Ok(Self::Close),
            "exec" => Ok(Self::Exec),
            "ask" => Ok(Self::Ask),
            "file" => Ok(Self::File),
            "edit" => Ok(Self::Edit(None)),
            "write" => Ok(Self::Write),
            "task" => Ok(Self::Task),
            "delegate" => Ok(Self::Delegate),
            _ => Err(format!("unknown block mode: {s}")),
        }
    }
}

impl BlockMode {
    #[must_use]
    pub fn requires_heredoc(&self) -> bool {
        matches!(self, BlockMode::Write | BlockMode::Edit(_))
    }

    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, BlockMode::Close | BlockMode::Task | BlockMode::Delegate)
    }
}

impl std::fmt::Display for BlockMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockMode::Watch => f.write_str("watch"),
            BlockMode::Close => f.write_str("close"),
            BlockMode::Exec => f.write_str("exec"),
            BlockMode::Ask => f.write_str("ask"),
            BlockMode::File => f.write_str("file"),
            BlockMode::Edit(_) => f.write_str("edit"),
            BlockMode::Write => f.write_str("write"),
            BlockMode::Task => f.write_str("task"),
            BlockMode::Delegate => f.write_str("delegate"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedBlock {
    pub window: String,
    pub mode: BlockMode,
    pub content: String,
    pub prose: Option<String>,
    #[serde(default)]
    pub dashboard: bool,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_str_watch() {
        assert_eq!("watch".parse::<BlockMode>(), Ok(BlockMode::Watch));
    }

    #[test]
    fn test_from_str_view_alias() {
        assert_eq!("view".parse::<BlockMode>(), Ok(BlockMode::Watch));
    }

    #[test]
    fn test_from_str_close() {
        assert_eq!("close".parse::<BlockMode>(), Ok(BlockMode::Close));
    }

    #[test]
    fn test_from_str_exec() {
        assert_eq!("exec".parse::<BlockMode>(), Ok(BlockMode::Exec));
    }

    #[test]
    fn test_from_str_ask() {
        assert_eq!("ask".parse::<BlockMode>(), Ok(BlockMode::Ask));
    }

    #[test]
    fn test_from_str_file() {
        assert_eq!("file".parse::<BlockMode>(), Ok(BlockMode::File));
    }

    #[test]
    fn test_from_str_edit() {
        let mode = "edit".parse::<BlockMode>().unwrap();
        assert!(matches!(mode, BlockMode::Edit(None)));
    }

    #[test]
    fn test_from_str_write() {
        assert_eq!("write".parse::<BlockMode>(), Ok(BlockMode::Write));
    }

    #[test]
    fn test_from_str_task() {
        assert_eq!("task".parse::<BlockMode>(), Ok(BlockMode::Task));
    }

    #[test]
    fn test_from_str_delegate() {
        assert_eq!("delegate".parse::<BlockMode>(), Ok(BlockMode::Delegate));
    }

    #[test]
    fn test_from_str_unknown() {
        assert!("unknown".parse::<BlockMode>().is_err());
    }

    #[test]
    fn test_from_str_empty() {
        assert!("".parse::<BlockMode>().is_err());
    }

    #[test]
    fn test_display() {
        assert_eq!(BlockMode::Watch.to_string(), "watch");
        assert_eq!(BlockMode::Close.to_string(), "close");
        assert_eq!(BlockMode::Exec.to_string(), "exec");
        assert_eq!(BlockMode::Ask.to_string(), "ask");
        assert_eq!(BlockMode::File.to_string(), "file");
        assert_eq!(BlockMode::Edit(None).to_string(), "edit");
        assert_eq!(BlockMode::Write.to_string(), "write");
        assert_eq!(BlockMode::Task.to_string(), "task");
        assert_eq!(BlockMode::Delegate.to_string(), "delegate");
    }

    #[test]
    fn test_requires_heredoc() {
        assert!(BlockMode::Write.requires_heredoc());
        assert!(BlockMode::Edit(None).requires_heredoc());
        assert!(!BlockMode::Watch.requires_heredoc());
        assert!(!BlockMode::Exec.requires_heredoc());
        assert!(!BlockMode::Ask.requires_heredoc());
        assert!(!BlockMode::File.requires_heredoc());
        assert!(!BlockMode::Close.requires_heredoc());
        assert!(!BlockMode::Task.requires_heredoc());
        assert!(!BlockMode::Delegate.requires_heredoc());
    }

    #[test]
    fn test_is_terminal() {
        assert!(BlockMode::Close.is_terminal());
        assert!(BlockMode::Task.is_terminal());
        assert!(BlockMode::Delegate.is_terminal());
        assert!(!BlockMode::Watch.is_terminal());
        assert!(!BlockMode::Exec.is_terminal());
        assert!(!BlockMode::Ask.is_terminal());
        assert!(!BlockMode::File.is_terminal());
        assert!(!BlockMode::Write.is_terminal());
        assert!(!BlockMode::Edit(None).is_terminal());
    }

    #[test]
    fn test_parsed_block_fields() {
        let block = ParsedBlock {
            window: "test".to_string(),
            mode: BlockMode::Exec,
            content: "cargo test".to_string(),
            prose: Some("running tests".to_string()),
            dashboard: true,
        };
        assert_eq!(block.window, "test");
        assert_eq!(block.mode, BlockMode::Exec);
        assert_eq!(block.content, "cargo test");
        assert_eq!(block.prose.as_deref(), Some("running tests"));
        assert!(block.dashboard);
    }
}
