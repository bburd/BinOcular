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

    #[error("missing dynamic offset reference `{field}`")]
    MissingOffsetReference { field: String },

    #[error("field `{field}` cannot be used as a dynamic offset source")]
    InvalidOffsetReferenceType { field: String },

    #[error("field `{field}` resolved to a negative dynamic offset")]
    NegativeOffsetReference { field: String },

    #[error("missing condition reference `{field}`")]
    MissingConditionReference { field: String },

    #[error("field `{field}` cannot be used as a condition source")]
    InvalidConditionReferenceType { field: String },

    #[error("field `{field}` resolved to a negative bit_set condition source")]
    NegativeBitSetConditionSource { field: String },

    #[error("missing expression reference `{field}`")]
    MissingExpressionReference { field: String },

    #[error("field `{field}` cannot be used as an expression source")]
    InvalidExpressionReferenceType { field: String },

    #[error("field `{field}` is too large to use in an expression")]
    ExpressionReferenceOverflow { field: String },

    #[error("expression arithmetic overflowed")]
    ExpressionOverflow,

    #[error("missing dynamic length reference `{field}`")]
    MissingLengthReference { field: String },

    #[error("field `{field}` cannot be used as a dynamic length source")]
    InvalidLengthReferenceType { field: String },

    #[error("field `{field}` resolved to a negative dynamic length")]
    NegativeLengthReference { field: String },

    #[error("field `{field}` resolved to a dynamic length that does not fit in usize")]
    LengthOverflow { field: String },

    #[error("expression resolved to a negative dynamic length")]
    NegativeExpressionLength,

    #[error("expression resolved to a zero dynamic length")]
    ZeroExpressionLength,

    #[error("expression resolved to a dynamic length that does not fit in usize")]
    ExpressionLengthOverflow,

    #[error("expression resolved to a negative dynamic offset")]
    NegativeExpressionOffset,

    #[error("relative offset cannot be resolved without a structure base")]
    RelativeOffsetWithoutBase,

    #[error("unknown structure `{name}`")]
    UnknownStructure { name: String },

    #[error("repeated structure `{name}` is missing an explicit stride")]
    MissingStructStride { name: String },

    #[error("unsupported field type in this core version")]
    Unsupported,
}
