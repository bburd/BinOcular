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

    #[error("resolved offset overflowed during repeat expansion")]
    OffsetOverflow,

    #[error("invalid numeric byte width: expected {expected}, got {actual}")]
    InvalidNumericByteWidth { expected: usize, actual: usize },

    #[error("missing dynamic length reference `{field}`")]
    MissingLengthReference { field: String },

    #[error("field `{field}` cannot be used as a dynamic length source")]
    InvalidLengthReferenceType { field: String },

    #[error("field `{field}` resolved to a negative dynamic length")]
    NegativeLengthReference { field: String },

    #[error("field `{field}` resolved to a dynamic length that does not fit in usize")]
    LengthOverflow { field: String },

    #[error("unsupported field type in this core version")]
    Unsupported,
}
