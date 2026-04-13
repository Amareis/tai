use async_trait::async_trait;
use tracing::{debug, warn};

use super::{Backend, CmdOutput};

pub struct LocalBackend;

impl Default for LocalBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalBackend {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Backend for LocalBackend {
    async fn run(&self, title: &str, command: &str) -> CmdOutput {
        debug!("run: executing '{title}': {command}");
        let full_script = format!("set -ex -o pipefail;\n{command}");

        let result = tokio::task::spawn_blocking(move || {
            match duct::cmd!("bash", "-c", &full_script)
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
