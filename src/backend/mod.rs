pub mod local;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub exit_code: i32,
    pub stdout: String,
}

const AWK: &str = r#"'{ if ($0 !~ /^[[:space:]]*$/) print "L" NR ": " $0 }'"#;

#[async_trait]
pub trait Backend: Send + Sync {
    async fn run(&self, title: &str, command: &str) -> CmdOutput;

    async fn file(&self, title: &str) -> CmdOutput {
        self.run(title, &format!(r#"awk {AWK} "{title}""#)).await
    }
}
