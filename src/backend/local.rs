use async_trait::async_trait;
use std::path::PathBuf;
use tracing::{debug, warn};

use super::{Backend, CmdOutput};

pub struct LocalBackend {
    cwd: PathBuf,
}

impl Default for LocalBackend {
    fn default() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

impl LocalBackend {
    #[must_use]
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[async_trait]
impl Backend for LocalBackend {
    async fn read_file(&self, path: &str) -> Result<String, std::io::Error> {
        let full_path = self.cwd.join(path);
        tokio::fs::read_to_string(&full_path).await
    }

    async fn run(&self, title: &str, command: &str) -> CmdOutput {
        debug!("run: executing '{title}': {command}");
        let full_script = format!("set -e -o pipefail;\n{command}");
        let cwd = self.cwd.clone();

        let result = tokio::task::spawn_blocking(move || {
            match duct::cmd!("bash", "-c", &full_script)
                .dir(&cwd)
                .stderr_to_stdout()
                .stdout_capture()
                .unchecked()
                .run()
            {
                Ok(output) => {
                    let exit_code = output.status.code().unwrap_or(1);
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    CmdOutput { exit_code, stdout }
                }
                Err(e) => CmdOutput {
                    exit_code: 1,
                    stdout: format!("EXECUTION ERROR: {e}"),
                },
            }
        })
        .await;

        match result {
            Ok(output) => {
                debug!("run: '{title}' done, exit={}", output.exit_code);
                output
            }
            Err(e) => {
                warn!("run: '{title}' task error: {e}");
                CmdOutput {
                    exit_code: 1,
                    stdout: format!("TASK ERROR: {e}"),
                }
            }
        }
    }
}
