use std::path::PathBuf;
use tokio::net::UnixListener;
use tracing::info;
use crate::core::CoreError;

pub struct RmFileOnDrop(PathBuf);

impl RmFileOnDrop {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        RmFileOnDrop(path)
    }
}

impl Drop for RmFileOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub async fn bind(socket_path: &PathBuf) -> Result<(UnixListener, RmFileOnDrop), CoreError> {
    if socket_path.exists() {
        tokio::fs::remove_file(&socket_path).await?;
    }
    
    let listener = UnixListener::bind(socket_path)?;
    info!("listening on {}", socket_path.display());

    Ok((listener, RmFileOnDrop::new(socket_path.clone())))
}