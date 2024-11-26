//! Encrypted FUSE filesystem.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Gocryptfs encrypted filesystem.
pub struct Crypt {
    storage_directory: PathBuf,
    mount_path: Option<PathBuf>,
}

impl Crypt {
    pub fn new(storage_directory: impl Into<PathBuf>, password: &str) -> io::Result<Self> {
        // Ensure target directory exists.
        let storage_directory = storage_directory.into();
        fs::create_dir_all(&storage_directory)?;

        // Initialize gocryptfs, if it's not a gocryptfs directory already.
        if !storage_directory.join("gocryptfs.conf").exists() {
            let mut gocryptfs = Command::new("gocryptfs")
                .arg("-q")
                .arg("-init")
                .arg(&storage_directory)
                .stdin(Stdio::piped())
                .spawn()?;
            gocryptfs.stdin.as_ref().unwrap().write_all(password.as_bytes())?;
            let status = gocryptfs.wait()?;

            if !status.success() {
                return Err(io::Error::other("Failed to initialize gocryptfs filesystem"));
            }
        }

        Ok(Self { storage_directory, mount_path: Default::default() })
    }

    /// Mount filesystem at the specified location.
    pub fn mount(&mut self, path: impl Into<PathBuf>, password: &str) -> io::Result<()> {
        let path = path.into();

        let mut gocryptfs = Command::new("gocryptfs")
            .arg("-q")
            .arg("-nonempty")
            .arg(&self.storage_directory)
            .arg(&path)
            .stdin(Stdio::piped())
            .spawn()?;
        gocryptfs.stdin.as_ref().unwrap().write_all(password.as_bytes())?;
        let status = gocryptfs.wait()?;

        if !status.success() {
            return Err(io::Error::other("Failed to mount gocryptfs filesystem"));
        }

        self.mount_path = Some(path);

        Ok(())
    }

    /// Unmount the filesystem.
    pub fn _unmount(&mut self) -> io::Result<()> {
        if let Some(mount_path) = self.mount_path.take() {
            // We need to call `umount` here because its SUID capabilities are required to
            // unmount the FUSE filesystem.
            let status = Command::new("umount").arg(&mount_path).spawn()?.wait()?;

            if !status.success() {
                return Err(io::Error::other("Failed to unmount gocryptfs filesystem"));
            }
        }

        Ok(())
    }
}
