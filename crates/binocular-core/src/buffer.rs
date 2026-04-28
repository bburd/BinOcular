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

#[cfg(test)]
mod tests {
    use super::{FileBuffer, MemoryBuffer, MmapBuffer};
    use crate::error::BufferError;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempTestFile {
        path: PathBuf,
    }

    impl TempTestFile {
        fn new(contents: &[u8]) -> Self {
            let mut path = std::env::temp_dir();
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock drifted before unix epoch")
                .as_nanos();
            path.push(format!(
                "binocular-core-buffer-test-{}-{timestamp}.bin",
                std::process::id()
            ));

            fs::write(&path, contents).expect("failed to write temp test file");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempTestFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn mmap_buffer_open_and_read_bytes_from_temp_file() {
        let bytes = [0x10, 0x20, 0x30, 0x40, 0x50];
        let file = TempTestFile::new(&bytes);

        let mmap = MmapBuffer::open(file.path()).expect("mmap open should succeed");

        assert_eq!(
            mmap.read_bytes(1, 3).expect("valid range should read"),
            &bytes[1..4]
        );
        assert_eq!(
            mmap.read_bytes(0, 0).expect("zero-length read should work"),
            &[]
        );
    }

    #[test]
    fn mmap_buffer_reports_correct_file_size() {
        let bytes = [1_u8, 2, 3, 4, 5, 6, 7];
        let file = TempTestFile::new(&bytes);

        let mmap = MmapBuffer::open(file.path()).expect("mmap open should succeed");

        assert_eq!(mmap.file_size(), bytes.len() as u64);
    }

    #[test]
    fn mmap_oob_errors_match_memory_buffer() {
        let data = vec![0xAA, 0xBB, 0xCC];
        let file = TempTestFile::new(&data);
        let mmap = MmapBuffer::open(file.path()).expect("mmap open should succeed");
        let memory = MemoryBuffer::from_vec(data);

        let cases = [
            (3_u64, 1_usize),       // starts at EOF with non-zero len
            (2_u64, 2_usize),       // overlaps beyond EOF
            (u64::MAX, 1_usize),    // offset cannot fit usize on some targets
            (1_u64, usize::MAX),    // addition overflow
            (u64::MAX, usize::MAX), // both problematic
        ];

        for (offset, len) in cases {
            assert!(
                matches!(
                    memory.read_bytes(offset, len),
                    Err(BufferError::OutOfBounds)
                ),
                "memory buffer should return OutOfBounds for offset={offset}, len={len}"
            );
            assert!(
                matches!(mmap.read_bytes(offset, len), Err(BufferError::OutOfBounds)),
                "mmap buffer should return OutOfBounds for offset={offset}, len={len}"
            );
        }
    }
}
