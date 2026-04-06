use crate::core::CoreError;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};
use tokio::net::{UnixListener, UnixStream};

pub struct Connection {
    reader: Lines<BufReader<ReadHalf<UnixStream>>>,
    writer: WriteHalf<UnixStream>,
}

impl Connection {
    pub async fn accept(listener: &UnixListener) -> Result<Self, CoreError> {
        let (stream, _addr) = listener.accept().await?;

        Ok(Self::from_stream(stream))
    }

    pub async fn connect(socket_path: &PathBuf) -> Result<Self, CoreError> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self::from_stream(stream))
    }

    pub fn from_stream(stream: UnixStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        let reader = BufReader::new(reader).lines();
        let writer = writer;
        Self { reader, writer }
    }

    pub async fn read_line(&mut self) -> Result<Option<String>, CoreError> {
        match self.reader.next_line().await {
            Ok(None) => Ok(None),
            Ok(Some(line)) => Ok(Some(line.trim().to_owned())),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn write_line(&mut self, text: &str) -> Result<(), CoreError> {
        self.writer.write_all(text.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}
