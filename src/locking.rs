//! Process locks for a catalogue's complete read/modify/write operation.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use crate::errors::{Error, Result};
use crate::pyerr;

/// Holds the OS lock on `<data>/.lamp.lock` until dropped. The file stays in
/// place: unlinking it would let another process lock a different inode.
/// The operating system releases the lock after a crash.
/// ponytail: writers share one lock per data directory. Seller-level locks
/// would also need a coordinated manifest writer if concurrent runs are needed.
pub struct CatalogueLock {
    file: File,
}

impl CatalogueLock {
    pub fn acquire(data_dir: &Path) -> Result<CatalogueLock> {
        let root = pyerr::resolve(data_dir);
        pyerr::mkdir_parents(&root).map_err(Error::Os)?;
        let path = root.join(".lamp.lock");
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|e| Error::Os(pyerr::os_error(&e, pyerr::Call::Open, &path)))?;
        let busy = || {
            Error::Busy(format!(
                "catalogue is already in use: {}",
                pyerr::path_str(&root)
            ))
        };
        if cfg!(windows) {
            // msvcrt.locking needs a byte to lock; Python writes one too.
            let len = file.seek(SeekFrom::End(0)).map_err(|_| busy())?;
            if len == 0 {
                file.write_all(b"\0").map_err(|_| busy())?;
                file.flush().map_err(|_| busy())?;
            }
        }
        file.try_lock().map_err(|_| busy())?;
        Ok(CatalogueLock { file })
    }
}

impl Drop for CatalogueLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
