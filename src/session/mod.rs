use crate::agent::AgentResponse;
use crate::backend::CmdOutput;
use crate::response::{parse_response, serialize_blocks};
use crate::types::{BlockMode, ParsedBlock};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::info;
use uuid::Uuid;

const SYSTEM_PROMPT: &str = include_str!("system_prompt.txt");
const INSTRUCTIONS: &str = include_str!("instructions.txt");
const DEFAULT_TAI: &str = include_str!("default_tai.md");

fn initial_tai_content(project_dir: &Path) -> String {
    std::fs::read_to_string(project_dir.join("tai.md"))
        .unwrap_or_else(|_| DEFAULT_TAI.to_string())
}

async fn read_task_interactive() -> Option<String> {
    let mut stdout = tokio::io::stdout();
    stdout.write_all(b"? What we do today?\n> ").await.ok()?;
    stdout.flush().await.ok()?;
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let line = lines.next_line().await.ok()??;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn read_task_stdin() -> Option<String> {
    let mut buf = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut tokio::io::stdin(), &mut buf)
        .await
        .ok()?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn read_task() -> Option<String> {
    if std::io::stdin().is_terminal() {
        read_task_interactive().await
    } else {
        read_task_stdin().await
    }
}

pub struct SessionDir {
    workspace: PathBuf,
    internal: PathBuf,
    project: PathBuf,
}

impl SessionDir {
    pub async fn create_new(
        project_dir: &Path,
        task: Option<&str>,
    ) -> Result<Self, std::io::Error> {
        let id = generate_short_id();
        let workspace = std::env::temp_dir().join(format!("tai-{id}"));

        let internal = workspace.join(".session");
        let session = Self {
            workspace,
            internal,
            project: project_dir.to_path_buf(),
        };

        session.init_session(project_dir, task).await?;
        info!("session created: {}", session.workspace.display());
        Ok(session)
    }
    async fn init_session(
        &self,
        project_dir: &Path,
        task: Option<&str>,
    ) -> Result<(), std::io::Error> {
        tokio::fs::create_dir_all(self.internal.join("out")).await?;
        tokio::fs::create_dir_all(self.internal.join("responses")).await?;

        let symlink = self.workspace.join("work");
        if !tokio::fs::try_exists(&symlink).await.unwrap_or(false) {
            tokio::fs::symlink(project_dir, &symlink).await?;
        }

        tokio::fs::write(self.internal.join("system-prompt.txt"), SYSTEM_PROMPT).await?;
        tokio::fs::write(self.internal.join("instructions.txt"), INSTRUCTIONS).await?;
        tokio::fs::write(self.internal.join("tick"), "0").await?;

        let tai_content = initial_tai_content(project_dir);
        let mut resp = parse_response(&tai_content);
        if let Some(t) = task {
            resp.task = t.to_string();
        } else if resp.task.is_empty()
            && let Some(t) = read_task().await {
                resp.task = t;
            }
        self.write_tick_response(0, &resp).await;

        Ok(())
    }

    pub async fn open(path: &Path) -> Result<Self, std::io::Error> {
        let internal = path.join(".session");
        if !tokio::fs::metadata(&internal)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("not a session directory: {}", path.display()),
            ));
        }

        let symlink = path.join("work");
        let project = if tokio::fs::symlink_metadata(&symlink)
            .await
            .map(|m| m.is_symlink())
            .unwrap_or(false)
        {
            tokio::fs::read_link(&symlink).await?
        } else {
            std::env::current_dir()?
        };

        info!("session opened: {}", path.display());
        Ok(Self {
            workspace: path.to_path_buf(),
            internal,
            project,
        })
    }

    pub async fn create_or_open(
        path: Option<&Path>,
        project_dir: &Path,
        task: Option<&str>,
    ) -> Result<Self, std::io::Error> {
        match path {
            Some(p)
                if tokio::fs::metadata(p.join(".session"))
                    .await
                    .map(|m| m.is_dir())
                    .unwrap_or(false) =>
            {
                Self::open(p).await
            }
            Some(p) => {
                let session = Self {
                    workspace: p.to_path_buf(),
                    internal: p.join(".session"),
                    project: project_dir.to_path_buf(),
                };
                session.init_session(project_dir, task).await?;
                info!("session created at: {}", p.display());
                Ok(session)
            }
            None => Self::create_new(project_dir, task).await,
        }
    }

    pub async fn read_index(&self) -> Vec<ParsedBlock> {
        let path = self.internal.join("index.md");
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => parse_response(&text).segments,
            Err(_) => Vec::new(),
        }
    }

    pub async fn write_index(&self, segments: &[ParsedBlock]) {
        let text = serialize_blocks(segments);
        let path = self.internal.join("index.md");
        tokio::fs::write(&path, text).await.ok();
    }

    pub async fn read_out(&self, title: &str) -> Option<CmdOutput> {
        let path = self.internal.join("out").join(format!("{title}.out"));
        let raw = tokio::fs::read_to_string(&path).await.ok()?;
        let (exit_code, stdout) = parse_out_file(&raw);
        Some(CmdOutput { exit_code, stdout })
    }

    pub async fn write_out(&self, title: &str, output: &CmdOutput) {
        let path = self.internal.join("out").join(format!("{title}.out"));
        let content = format!("exit {}\n{}", output.exit_code, output.stdout);
        tokio::fs::write(&path, content).await.ok();
    }

    pub async fn remove_out(&self, title: &str) {
        let path = self.internal.join("out").join(format!("{title}.out"));
        tokio::fs::remove_file(&path).await.ok();
    }

    #[must_use]
    pub async fn has_out(&self, title: &str) -> bool {
        tokio::fs::try_exists(self.internal.join("out").join(format!("{title}.out")))
            .await
            .unwrap_or(false)
    }

    pub async fn read_tick(&self, default: u64) -> u64 {
        let path = self.internal.join("tick");
        tokio::fs::read_to_string(&path)
            .await
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(default)
    }

    pub async fn write_tick(&self, n: u64) {
        let path = self.internal.join("tick");
        tokio::fs::write(&path, n.to_string()).await.ok();
    }

    pub async fn read_system_prompt(&self) -> String {
        let path = self.internal.join("system-prompt.txt");
        tokio::fs::read_to_string(&path)
            .await
            .unwrap_or_else(|_| SYSTEM_PROMPT.to_string())
    }

    pub async fn read_instructions(&self) -> String {
        let path = self.internal.join("instructions.txt");
        tokio::fs::read_to_string(&path)
            .await
            .unwrap_or_else(|_| INSTRUCTIONS.to_string())
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    #[must_use]
    pub fn session_path(&self) -> &Path {
        &self.workspace
    }

    #[must_use]
    pub fn internal_path(&self) -> &Path {
        &self.internal
    }

    #[must_use]
    pub fn project(&self) -> &Path {
        &self.project
    }

    pub async fn read_all_outputs(
        &self,
        segments: &[ParsedBlock],
    ) -> HashMap<String, CmdOutput> {
        let mut outputs = HashMap::new();
        for block in segments {
            if block.mode == BlockMode::Close {
                continue;
            }
            if let Some(output) = self.read_out(&block.window).await {
                outputs.insert(block.window.clone(), output);
            }
        }
        outputs
    }

    pub async fn read_mind(&self) -> String {
        let path = self.internal.join("mind.md");
        tokio::fs::read_to_string(&path).await.unwrap_or_default()
    }

    pub async fn write_mind(&self, steps: &str) {
        let path = self.internal.join("mind.md");
        tokio::fs::write(&path, steps).await.ok();
    }

    pub async fn write_tick_response(&self, tick_n: u64, response: &AgentResponse) {
        let dir = self.internal.join("responses");
        tokio::fs::create_dir_all(&dir).await.ok();
        let path = dir.join(format!("{tick_n:0>5}.toml"));
        tokio::fs::write(
            &path,
            toml::to_string(response).unwrap_or_else(|e| e.to_string()),
        )
        .await
        .ok();
    }

    pub async fn read_tick_response(&self, tick_n: u64) -> Option<AgentResponse> {
        let path = self
            .internal
            .join("responses")
            .join(format!("{tick_n:0>5}.toml"));
        toml::from_str(&tokio::fs::read_to_string(path).await.ok()?).ok()
    }

    pub async fn read_response_history(&self, from_tick: u64) -> Vec<(u64, AgentResponse)> {
        let dir = self.internal.join("responses");
        let mut result = Vec::new();
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            return result;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let tick: u64 = match name_str.strip_suffix(".toml").and_then(|s| s.parse().ok()) {
                Some(t) => t,
                None => continue,
            };
            if tick < from_tick {
                continue;
            }
            if let Ok(content) = tokio::fs::read_to_string(entry.path()).await
                && let Ok(resp) = toml::from_str::<AgentResponse>(&content)
            {
                result.push((tick, resp));
            }
        }
        result.sort_by_key(|(t, _)| *t);
        result
    }

    pub fn print_banner(&self) {
        let to_resume = format!("  tai {}  ", self.session_path().display());
        let width = std::cmp::max(50, to_resume.chars().count());
        let border = "═".repeat(width);
        println!();
        println!("╔{:═^width$}╗", " TO RESUME SESSION ");
        println!("║{:^width$}║", " ");
        println!("║{to_resume:^width$}║");
        println!("║{:^width$}║", " ");
        println!("╚{border}╝");
    }
}

fn parse_out_file(raw: &str) -> (i32, String) {
    let first_line = raw.lines().next().unwrap_or("exit 0");
    let rest = if raw.len() > first_line.len() + 1 {
        raw[first_line.len() + 1..].to_string()
    } else {
        String::new()
    };

    let exit_code = if let Some(code_str) = first_line.strip_prefix("exit ") {
        code_str.trim().parse::<i32>().unwrap_or(0)
    } else if let Some(sig_str) = first_line.strip_prefix("signal ") {
        sig_str.trim().parse::<i32>().unwrap_or(-1)
    } else {
        0
    };

    (exit_code, rest)
}

const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

fn generate_short_id() -> String {
    let uuid = Uuid::new_v4();
    let bytes = uuid.as_bytes();
    bytes
        .iter()
        .take(6)
        .map(|b| {
            let idx = usize::from(*b) % CHARSET.len();
            char::from(CHARSET.get(idx).copied().unwrap_or(b'a'))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_out_file_exit() {
        let (code, stdout) = parse_out_file("exit 0\nhello\nworld");
        assert_eq!(code, 0);
        assert_eq!(stdout, "hello\nworld");
    }

    #[test]
    fn test_parse_out_file_error() {
        let (code, stdout) = parse_out_file("exit 1\nerror msg");
        assert_eq!(code, 1);
        assert_eq!(stdout, "error msg");
    }

    #[test]
    fn test_parse_out_file_signal() {
        let (code, _stdout) = parse_out_file("signal 9\n");
        assert_eq!(code, 9);
    }

    #[test]
    fn test_parse_out_file_empty() {
        let (code, stdout) = parse_out_file("exit 0\n");
        assert_eq!(code, 0);
        assert_eq!(stdout, "");
    }
}
