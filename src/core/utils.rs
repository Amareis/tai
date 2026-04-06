use std::path::PathBuf;

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
