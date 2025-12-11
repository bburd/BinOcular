use thiserror::Error;

#[derive(Debug, Error)]
pub enum BufferError {
    #[error("read out of bounds")]
    OutOfBounds,
}

#[derive(Debug, Error)]
pub enum InterpretError {
    #[error("buffer error: {0}")]
    Buffer(#[from] BufferError),

    #[error("unsupported field type in this core version")]
    Unsupported,
}
