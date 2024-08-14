#[derive(Debug, thiserror::Error)]
pub enum OpenTsConfigError {
    #[error("Serde deserialization error: {0}")]
    SerdeError(serde_json::Error),
    #[error("Disk I/O Error: {0}")]
    IOError(std::io::Error),
}
