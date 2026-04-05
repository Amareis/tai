pub mod terminal_manager;

use std::fs::File;
use std::os::fd::{FromRawFd, IntoRawFd};
use std::path::PathBuf;

use roam_fdpass::{recv_fd, send_fd};
use thiserror::Error;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot::Sender;
use tracing::{info};

#[derive(Debug, Error)]
pub enum FdError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to receive FD: {0}")]
    RecvFailed(String),

    #[error("failed to send FD: {0}")]
    SendFailed(String),
}

/// Идентификатор viewport — User (stdout сервера) или Model (FD от клиента).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewId {
    User,
    Model,
}

/// Listen на Unix сокете, дождаться одного подключения, получить FD.
pub fn recv_connection(socket_path: PathBuf, sender: Sender<Result<File, FdError>>) {
    tokio::spawn(async move {
        let _ = sender.send(recv_connection_inner(socket_path).await);
    });
}

pub async fn recv_connection_inner(socket_path: PathBuf) -> Result<File, FdError> {
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    info!("listening for FD on {}", socket_path.display());

    let (stream, _addr) = listener.accept().await?;
    info!("client connected, receiving FD...");

    let fd = recv_fd(&stream)
        .await
        .map_err(|e| FdError::RecvFailed(format!("{e}")))?;

    let file = unsafe { File::from_raw_fd(fd) };
    info!("received FD: {fd}");

    let _ = std::fs::remove_file(&socket_path);
    Ok(file)
}

/// Подключиться к Unix сокету и передать stdin/stdout как FD.
///
/// После передачи процесс засыпает. SIGWINCH пересылается как resize-сигнал.
pub async fn send_connection(socket_path: PathBuf) -> Result<(), FdError> {
    info!("connecting to {}...", socket_path.display());

    let stream = UnixStream::connect(&socket_path).await?;
    info!("connected, sending stdout FD...");

    let stdin_file = File::open("/dev/stdout")?;
    let raw_fd = stdin_file.into_raw_fd();

    send_fd(&stream, raw_fd)
        .await
        .map_err(|e| FdError::SendFailed(format!("{e}")))?;

    info!("FD sent");

    Ok(())
}
