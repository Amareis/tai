use crate::agent::AgentResponse;
use crate::backend::CmdOutput;
use crate::response::{parse_response, serialize_blocks};
use crate::types::ParsedBlock;
use std::path::{Path, PathBuf};
use tracing::info;
use uuid::Uuid;

const SYSTEM_PROMPT: &str = include_str!("system_prompt.txt");
const DEFAULT_TAI: &str = include_str!("default_tai.md");

pub struct SessionDir {
    workspace: PathBuf,
    internal: PathBuf,
    project: PathBuf,
}

impl SessionDir {
    pub fn create_new(project_dir: &Path, debug: bool) -> Result<Self, std::io::Error> {
        let id = generate_short_id();
        let workspace = if debug {
            project_dir.join(".tai").join(&id)
        } else {
            std::env::temp_dir().join(format!("tai-{id}"))
        };

        let internal = workspace.join(".session");
        std::fs::create_dir_all(internal.join("out"))?;
        std::fs::create_dir_all(internal.join("responses"))?;

        let symlink = workspace.join("work");
        if !symlink.exists() {
            std::os::unix::fs::symlink(project_dir, &symlink)?;
        }

        std::fs::write(internal.join("system-prompt.txt"), SYSTEM_PROMPT)?;
        std::fs::write(internal.join("tick"), "0")?;

        let session = Self {
            workspace,
            internal,
            project: project_dir.to_path_buf(),
        };

        let tai_content = std::fs::read_to_string(project_dir.join("tai.md"))
            .unwrap_or_else(|_| DEFAULT_TAI.to_string());
        let resp = parse_response(&tai_content);
        let next_steps = resp.next_steps.clone();
        session.write_response(&resp);
        session.write_next_steps(&next_steps);

        info!("session created: {}", session.workspace.display());
        Ok(session)
    }

    pub fn open(path: &Path) -> Result<Self, std::io::Error> {
        let internal = path.join(".session");
        if !internal.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("not a session directory: {}", path.display()),
            ));
        }

        let symlink = path.join("work");
        let project = if symlink.is_symlink() {
            std::fs::read_link(&symlink)?
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

    pub fn create_or_open(
        path: Option<&Path>,
        project_dir: &Path,
        debug: bool,
    ) -> Result<Self, std::io::Error> {
        match path {
            Some(p) if p.join(".session").is_dir() => Self::open(p),
            Some(p) => {
                let session = Self {
                    workspace: p.to_path_buf(),
                    internal: p.join(".session"),
                    project: project_dir.to_path_buf(),
                };
                std::fs::create_dir_all(session.internal.join("out"))?;
                std::fs::create_dir_all(session.internal.join("responses"))?;
                let symlink = session.workspace.join("work");
                if !symlink.exists() {
                    std::os::unix::fs::symlink(project_dir, &symlink)?;
                }
                std::fs::write(session.internal.join("system-prompt.txt"), SYSTEM_PROMPT)?;
                std::fs::write(session.internal.join("tick"), "0")?;
                let tai_content = std::fs::read_to_string(project_dir.join("tai.md"))
                    .unwrap_or_else(|_| DEFAULT_TAI.to_string());
                let resp = parse_response(&tai_content);
                let next_steps = resp.next_steps.clone();
                session.write_response(&resp);
                session.write_next_steps(&next_steps);
                info!("session created at: {}", p.display());
                Ok(session)
            }
            None => Self::create_new(project_dir, debug),
        }
    }

    #[must_use]
    pub fn read_index(&self) -> Vec<ParsedBlock> {
        let path = self.internal.join("index.md");
        match std::fs::read_to_string(&path) {
            Ok(text) => parse_response(&text).segments,
            Err(_) => Vec::new(),
        }
    }

    pub fn write_index(&self, segments: &[ParsedBlock]) {
        let text = serialize_blocks(segments);
        let path = self.internal.join("index.md");
        std::fs::write(&path, text).ok();
    }

    #[must_use]
    pub fn read_out(&self, title: &str) -> Option<CmdOutput> {
        let path = self.internal.join("out").join(format!("{title}.out"));
        let raw = std::fs::read_to_string(&path).ok()?;
        let (exit_code, stdout) = parse_out_file(&raw);
        Some(CmdOutput { exit_code, stdout })
    }

    pub fn write_out(&self, title: &str, output: &CmdOutput) {
        let path = self.internal.join("out").join(format!("{title}.out"));
        let content = format!("exit {}\n{}", output.exit_code, output.stdout);
        std::fs::write(&path, content).ok();
    }

    pub fn remove_out(&self, title: &str) {
        let path = self.internal.join("out").join(format!("{title}.out"));
        std::fs::remove_file(&path).ok();
    }

    #[must_use]
    pub fn has_out(&self, title: &str) -> bool {
        self.internal
            .join("out")
            .join(format!("{title}.out"))
            .exists()
    }

    #[must_use]
    pub async fn read_tick(&self, default: u64) -> u64 {
        let path = self.internal.join("tick");
        tokio::fs::read_to_string(&path)
            .await
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(default)
    }

    pub fn write_tick(&self, n: u64) {
        let path = self.internal.join("tick");
        std::fs::write(&path, n.to_string()).ok();
    }

    #[must_use]
    pub async fn read_system_prompt(&self) -> String {
        let path = self.internal.join("system-prompt.txt");
        tokio::fs::read_to_string(&path)
            .await
            .unwrap_or_else(|_| SYSTEM_PROMPT.to_string())
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
    pub fn project(&self) -> &Path {
        &self.project
    }

    #[must_use]
    pub fn read_all_outputs(
        &self,
        segments: &[ParsedBlock],
    ) -> std::collections::HashMap<String, CmdOutput> {
        let mut outputs = std::collections::HashMap::new();
        for block in segments {
            if block.mode == crate::types::BlockMode::Close {
                continue;
            }
            if let Some(output) = self.read_out(&block.window) {
                outputs.insert(block.window.clone(), output);
            }
        }
        outputs
    }

    pub fn write_response(&self, response: &AgentResponse) {
        let path = self.internal.join("response.toml");
        std::fs::write(
            &path,
            toml::to_string(response).unwrap_or_else(|e| e.to_string()),
        )
        .ok();
    }
    #[must_use]
    pub fn read_response(&self) -> Option<AgentResponse> {
        let path = self.internal.join("response.toml");
        toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    #[must_use]
    pub fn read_next_steps(&self) -> String {
        let path = self.internal.join("next-steps.md");
        std::fs::read_to_string(&path).unwrap_or_default()
    }

    pub fn write_next_steps(&self, steps: &str) {
        let path = self.internal.join("next-steps.md");
        std::fs::write(&path, steps).ok();
    }

    pub fn write_tick_response(&self, tick_n: u64, response: &AgentResponse) {
        let dir = self.internal.join("responses");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(format!("{tick_n}.toml"));
        std::fs::write(
            &path,
            toml::to_string(response).unwrap_or_else(|e| e.to_string()),
        )
        .ok();
    }

    #[must_use]
    pub fn read_tick_response(&self, tick_n: u64) -> Option<AgentResponse> {
        let path = self
            .internal
            .join("responses")
            .join(format!("{tick_n}.toml"));
        toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    #[must_use]
    pub fn read_response_history(&self, from_tick: u64) -> Vec<(u64, AgentResponse)> {
        let dir = self.internal.join("responses");
        let mut result = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return result;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let tick: u64 = match name_str.strip_suffix(".toml").and_then(|s| s.parse().ok()) {
                Some(t) => t,
                None => continue,
            };
            if tick < from_tick {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(entry.path())
                && let Ok(resp) = toml::from_str::<AgentResponse>(&content)
            {
                result.push((tick, resp));
            }
        }
        result.sort_by_key(|(t, _)| *t);
        result
    }

    pub fn print_banner(&self) {
        let to_resume = format!("  tai server {}  ", self.session_path().display());
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
