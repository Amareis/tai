pub mod local;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub exit_code: i32,
    pub stdout: String,
}

#[async_trait]
pub trait Backend: Send + Sync {
    async fn run(&self, title: &str, command: &str) -> CmdOutput;

    async fn file(&self, title: &str) -> CmdOutput {
        self.run(title, &format!(r#"awk '{{print "L" NR ": " $0}}' "{title}""#)).await
    }
}
