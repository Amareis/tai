use super::Connection;

pub struct ModelView {
    conn: Connection,
}

impl ModelView {
    #[must_use]
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    pub async fn write_state_header(
        &mut self,
        active_windows: usize,
        frozen_windows: usize,
        tokens: usize,
        focus: Option<&str>,
    ) -> Result<(), super::CoreError> {
        let separator = "═".repeat(60);
        self.conn.write_line(&separator).await?;

        self.conn
            .write_line(&format!(
                "STATE: Active {active_windows} | Frozen {frozen_windows} | Tokens: {tokens}"
            ))
            .await?;

        if let Some(focus_name) = focus {
            self.conn
                .write_line(&format!("FOCUS: {focus_name}"))
                .await?;
        }

        self.conn.write_line(&separator).await?;
        Ok(())
    }

    pub async fn write_command_result(
        &mut self,
        window_name: &str,
        output: &str,
        exit_code: Option<i32>,
    ) -> Result<(), super::CoreError> {
        self.conn.write_line(&format!("[{window_name}]")).await?;
        self.conn.write_line(output).await?;

        if let Some(code) = exit_code {
            self.conn.write_line(&format!("exit code: {code}")).await?;
        }

        self.conn.write_line("").await?;
        Ok(())
    }

    pub async fn write_prompt(&mut self) -> Result<(), super::CoreError> {
        self.conn.write_line("> ").await?;
        Ok(())
    }

    pub async fn write_text(&mut self, text: &str) -> Result<(), super::CoreError> {
        self.conn.write_line(text).await?;
        Ok(())
    }

    pub async fn write_empty_line(&mut self) -> Result<(), super::CoreError> {
        self.conn.write_line("").await?;
        Ok(())
    }

    pub fn connection(&mut self) -> &mut Connection {
        &mut self.conn
    }
}
