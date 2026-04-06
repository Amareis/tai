use super::CoreError;
use crate::core::connection::Connection;
use rustyline_async::{Readline, ReadlineError, ReadlineEvent, SharedWriter};
use std::io::Write;
use std::path::PathBuf;
use tokio::select;
use tracing::info;

pub struct Client {
    rl: Readline,
    out: SharedWriter,
    server: Connection,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.rl.flush();
    }
}

impl Client {
    pub async fn connect(socket_path: PathBuf) -> Result<Self, CoreError> {
        info!("connecting to {}...", socket_path.display());
        let server = Connection::connect(&socket_path).await?;
        info!("connected");

        let (rl, out) = Readline::new("> ".to_string())?;
        Ok(Client { rl, out, server })
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        self.rl.should_print_line_on(true, false);

        loop {
            select! {
                event = self.rl.readline() => {
                    match event {
                        Ok(ReadlineEvent::Line(line)) => {
                            let trimmed = line.trim();
                            self.rl.add_history_entry(trimmed.to_owned());
                            self.server.write_line(trimmed).await?;
                        }
                        Ok(ReadlineEvent::Eof) | Err(ReadlineError::Closed) => break,
                        Ok(ReadlineEvent::Interrupted) => {
                            writeln!(self.out, "^C")?;
                        }
                        Err(e) => return Err(CoreError::Readline(e)),
                    }
                }
                line = self.server.read_line() => {
                    match line {
                        Ok(Some(output)) => {
                            writeln!(self.out, "{output}")?;
                        }
                        Ok(None) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }

        Ok(())
    }
}
