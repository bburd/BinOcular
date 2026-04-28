use std::{fs::File, path::Path};

use crate::error::BufferError;
use memmap2::{Mmap, MmapOptions};

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

pub struct MmapBuffer {
    mmap: Mmap,
}

impl MmapBuffer {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let file = File::open(path)?;

        // SAFETY: The mapping is read-only and we do not mutate the underlying
        // file through this handle, so aliasing requirements are respected.
        let mmap = unsafe { MmapOptions::new().map(&file)? };

        Ok(Self { mmap })
    }
}

impl FileBuffer for MemoryBuffer {
    fn file_size(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<&[u8], BufferError> {
        let (offset, end) = checked_range(self.data.len(), offset, len)?;
        Ok(&self.data[offset..end])
    }
}

impl FileBuffer for MmapBuffer {
    fn file_size(&self) -> u64 {
        self.mmap.len() as u64
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<&[u8], BufferError> {
        let (offset, end) = checked_range(self.mmap.len(), offset, len)?;
        Ok(&self.mmap[offset..end])
    }
}

fn checked_range(data_len: usize, offset: u64, len: usize) -> Result<(usize, usize), BufferError> {
    let offset = usize::try_from(offset).map_err(|_| BufferError::OutOfBounds)?;
    let end = offset.checked_add(len).ok_or(BufferError::OutOfBounds)?;

    if end > data_len {
        return Err(BufferError::OutOfBounds);
    }

    Ok((offset, end))
}
