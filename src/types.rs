use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockMode {
    Watch,
    Close,
    Exec,
    Ask,
    File,
    Edit,
    Write,
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
            "edit" => Ok(Self::Edit),
            "write" => Ok(Self::Write),
            _ => Err(format!("unknown block mode: {s}")),
        }
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
            BlockMode::Edit => f.write_str("edit"),
            BlockMode::Write => f.write_str("write"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedBlock {
    pub window: String,
    pub mode: BlockMode,
    pub content: String,
    pub prose: Option<String>,
}
