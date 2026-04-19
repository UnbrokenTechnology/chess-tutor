use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid FEN: {0}")]
    InvalidFen(String),

    #[error("opening book error: {0}")]
    Book(String),

    #[error("engine error: {0}")]
    Engine(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
