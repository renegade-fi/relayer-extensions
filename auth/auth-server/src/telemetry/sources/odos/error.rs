use thiserror::Error;

#[derive(Debug, Error)]
pub enum OdosError {
    #[error("Invalid input: {0}")]
    Input(String),
}
