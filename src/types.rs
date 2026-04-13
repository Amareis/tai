use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockMode {
    View,
    Close,
    Exec,
}

impl std::str::FromStr for BlockMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "close" => Ok(Self::Close),
            "view" | "text" => Ok(Self::View),
            "exec" | "write" => Ok(Self::Exec),
            _ => Err(format!("unknown block mode: {s}")),
        }
    }
}

impl std::fmt::Display for BlockMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockMode::View => f.write_str("view"),
            BlockMode::Close => f.write_str("close"),
            BlockMode::Exec => f.write_str("exec"),
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
