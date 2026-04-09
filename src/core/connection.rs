use crate::backend::BackendError::Communication;
use crate::core::CoreError;
use crate::core::utils::{RmFileOnDrop, sleep_some_or_forever};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::select;

pub struct Connection {
    reader: Lines<BufReader<ReadHalf<UnixStream>>>,
    writer: WriteHalf<UnixStream>,
    _socket_guard: Option<RmFileOnDrop>,
}

impl Connection {
    pub async fn accept(
        listener: &UnixListener,
        guard: Option<RmFileOnDrop>,
        timeout: Option<Duration>,
    ) -> Result<Self, CoreError> {
        select! {
            res = listener.accept() => {
                Ok(Self::from_stream(res?.0, guard))
            }
            _timeout = sleep_some_or_forever(timeout) => {
                Err(CoreError::Backend(Communication("client connect timeout reached".into())))
            }
        }
    }

    pub async fn connect(socket_path: &PathBuf) -> Result<Self, CoreError> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self::from_stream(stream, None))
    }

    pub fn from_stream(stream: UnixStream, guard: Option<RmFileOnDrop>) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        let reader = BufReader::new(reader).lines();
        let writer = writer;
        Self {
            reader,
            writer,
            _socket_guard: guard,
        }
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
