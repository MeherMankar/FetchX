// Adapted from ffmpreg (Apache-2.0)
#![allow(clippy::module_inception, clippy::collapsible_if)]

pub mod codecs;
pub mod container;
pub mod frame;
pub mod metadata;
pub mod packet;
pub mod state;
pub mod stream;
pub mod time;
pub mod traits;

pub use frame::*;
pub use metadata::*;
pub use packet::*;
pub use state::*;
pub use stream::*;
pub use time::*;
pub use traits::*;

use std::path::Path;

// ---------------------------------------------------------------------------
// Error / Result
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Message {
    Io(std::io::Error),
    Other(&'static str),
    Container(&'static str),
    Codec(&'static str),
}

pub type Result<T> = std::result::Result<T, Message>;

impl From<std::io::Error> for Message {
    fn from(err: std::io::Error) -> Self {
        Message::Io(err)
    }
}

impl From<&'static str> for Message {
    fn from(err: &'static str) -> Self {
        Message::Other(err)
    }
}

// ---------------------------------------------------------------------------
// Io trait
// ---------------------------------------------------------------------------

pub trait Io {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut slice = buf;
        while !slice.is_empty() {
            let n = self.write(slice)?;
            if n == 0 {
                return Err("short write".into());
            }
            slice = &slice[n..];
        }
        Ok(())
    }

    #[allow(unused_variables)]
    fn seek(&mut self, pos: u64) -> Result<()> {
        Err("seek not supported on this backend".into())
    }

    fn size(&self) -> Result<u64> {
        Err("size not supported on this backend".into())
    }
}

// ---------------------------------------------------------------------------
// File-backed Io implementation
// ---------------------------------------------------------------------------

pub struct IoFile {
    file: std::fs::File,
}

impl IoFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(path.as_ref())?;
        Ok(Self { file })
    }

    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::create(path.as_ref())?;
        Ok(Self { file })
    }
}

impl Io for IoFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        use std::io::Read;
        Ok(self.file.read(buf)?)
    }

    fn seek(&mut self, position: u64) -> Result<()> {
        use std::io::{Seek, SeekFrom};
        self.file.seek(SeekFrom::Start(position))?;
        Ok(())
    }

    fn size(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        use std::io::Write;
        Ok(self.file.write(buf)?)
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        use std::io::Write;
        self.file.write_all(buf)?;
        Ok(())
    }
}
