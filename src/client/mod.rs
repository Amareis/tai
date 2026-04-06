pub mod model_view;

use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tracing::info;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("connection closed")]
    ConnectionClosed,
}

pub struct Server {
    listener: UnixListener,
    socket_path: PathBuf,
}

pub struct ServerConnection {
    reader: BufReader<ReadHalf<UnixStream>>,
    writer: Arc<Mutex<WriteHalf<UnixStream>>>,
}

impl Server {
    pub async fn bind(socket_path: PathBuf) -> Result<Self, ClientError> {
        if socket_path.exists() {
            tokio::fs::remove_file(&socket_path).await?;
        }

        let listener = UnixListener::bind(&socket_path)?;
        info!("listening on {}", socket_path.display());

        Ok(Self {
            listener,
            socket_path,
        })
    }

    pub async fn accept(self) -> Result<(ServerConnection, PathBuf), ClientError> {
        let (stream, _addr) = self.listener.accept().await?;
        info!("client connected");

        let (reader, writer) = tokio::io::split(stream);
        let reader = BufReader::new(reader);
        let writer = Arc::new(Mutex::new(writer));

        Ok((
            ServerConnection { reader, writer },
            self.socket_path.clone(),
        ))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl ServerConnection {
    pub async fn read_line(&mut self) -> Result<Option<String>, ClientError> {
        let mut line = String::new();
        match self.reader.read_line(&mut line).await {
            Ok(0) => Ok(None),
            Ok(_) => {
                let trimmed = line
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_string();
                Ok(Some(trimmed))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub async fn write_line(&self, text: &str) -> Result<(), ClientError> {
        let mut writer = self.writer.lock().await;
        writer.write_all(text.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    pub async fn write(&self, text: &str) -> Result<(), ClientError> {
        let mut writer = self.writer.lock().await;
        writer.write_all(text.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }
}

pub async fn run_client(socket_path: PathBuf) -> Result<(), ClientError> {
    info!("connecting to {}...", socket_path.display());

    let stream = UnixStream::connect(&socket_path).await?;
    info!("connected");

    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader).lines();
    let stdin = tokio::io::stdin();
    let mut stdin_lines = BufReader::new(stdin).lines();

    let mut stdout = tokio::io::stdout();

    loop {
        tokio::select! {
            line = stdin_lines.next_line() => {
                match line {
                    Ok(Some(input)) => {
                        writer.write_all(input.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;
                    }
                    Ok(None) => break,
                    Err(e) => return Err(e.into()),
                }
            }
            line = reader.next_line() => {
                match line {
                    Ok(Some(output)) => {
                        stdout.write_all(output.as_bytes()).await?;
                        stdout.write_all(b"\n").await?;
                        stdout.flush().await?;
                    }
                    Ok(None) => break,
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    Ok(())
}
