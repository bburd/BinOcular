use crate::error::BufferError;

pub trait FileBuffer: Send + Sync {
    fn file_size(&self) -> u64;
    fn read_bytes(&self, offset: u64, len: usize) -> Result<&[u8], BufferError>;
}

pub struct MemoryBuffer {
    data: Vec<u8>,
}

impl MemoryBuffer {
    pub fn from_vec(data: Vec<u8>) -> Self {
        Self { data }
    }
}

impl FileBuffer for MemoryBuffer {
    fn file_size(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<&[u8], BufferError> {
        let offset = offset as usize;
        let end = offset.checked_add(len).ok_or(BufferError::OutOfBounds)?;
        if end > self.data.len() {
            return Err(BufferError::OutOfBounds);
        }
        Ok(&self.data[offset..end])
    }
}
