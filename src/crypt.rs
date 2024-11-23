//! File-backed encrypted mount.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::mem::MaybeUninit;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;

use rustix::fs::{self, FallocateFlags};
use rustix::mount::{self, MountFlags, UnmountFlags};
use uuid::Uuid;

use crate::libcryptsetup;

pub struct Crypt {
    crypt_device: *mut libcryptsetup::crypt_device,
    /// Mapped device name with trailing \0.
    mapped_name: Option<(String, CString)>,
    mount_path: Option<PathBuf>,
}

impl Crypt {
    /// Create a new encrypted file.
    pub fn new(path: impl Into<PathBuf>, size: u64) -> Result<Self, crate::Error> {
        // See https://mbroz.fedorapeople.org/libcryptsetup_API.

        let path = path.into();

        let passphrase = c"todo:pass"; // TODO

        if path.exists() {
            // Initialize crypt device.
            let mut crypt = Self::init(&path)?;

            // Load LUKS headers.
            crypt.load_luks()?;

            // Map the crypt device.
            crypt.map(passphrase)?;

            Ok(crypt)
        } else {
            // Create a new file with the desired size.
            let file = File::create_new(&path)?;
            fs::fallocate(file.as_fd(), FallocateFlags::empty(), 0, size)?;

            // Initialize crypt device.
            let mut crypt = Self::init(&path)?;

            // Setup LUKS.
            crypt.setup_encryption(passphrase)?;

            // Map the crypt device.
            crypt.map(passphrase)?;

            // Create ext4 filesystem.
            crypt.mkfs_ext4()?;

            Ok(crypt)
        }
    }

    /// Mount filesystem at `path`.
    pub fn mount(&mut self, path: impl Into<PathBuf>) -> Result<(), crate::Error> {
        let mapped_name = match &self.mapped_name {
            Some((mapped_name, _)) => mapped_name,
            None => return Err(Error::Unmapped.into()),
        };

        let path = path.into();
        let mapper_path = PathBuf::from("/dev/mapper").join(mapped_name);
        mount::mount2(Some(mapper_path), &path, Some("ext4"), MountFlags::BIND, None)?;

        self.mount_path = Some(path);

        Ok(())
    }

    /// Initialize a crypt device for the specified path.
    fn init(path: &Path) -> Result<Self, crate::Error> {
        let c_path = CString::new(path.as_os_str().as_encoded_bytes())?;
        let mut crypt_device: MaybeUninit<*mut libcryptsetup::crypt_device> = MaybeUninit::uninit();

        let result =
            unsafe { libcryptsetup::crypt_init(crypt_device.as_mut_ptr(), c_path.as_ptr()) };
        if result < 0 {
            return Err(Error::Init.into());
        }

        Ok(Self {
            crypt_device: unsafe { crypt_device.assume_init() },
            mapped_name: Default::default(),
            mount_path: Default::default(),
        })
    }

    /// Load existing LUKS headers.
    fn load_luks(&self) -> Result<(), Error> {
        let result = unsafe {
            libcryptsetup::crypt_load(self.crypt_device, c"LUKS2".as_ptr(), ptr::null_mut())
        };
        if result < 0 {
            return Err(Error::Load);
        }
        Ok(())
    }

    /// Setup LUKS encryption.
    fn setup_encryption(&self, passphrase: &CStr) -> Result<(), Error> {
        // Add LUKS header to the file.
        let result = unsafe {
            libcryptsetup::crypt_format(
                self.crypt_device,
                c"LUKS2".as_ptr(),
                c"aes".as_ptr(),
                c"xts-plain64".as_ptr(),
                ptr::null(),
                ptr::null(),
                512 / 8,
                ptr::null_mut(),
            )
        };
        if result < 0 {
            return Err(Error::Format);
        }

        // Add a keyslot for password.
        let result = unsafe {
            libcryptsetup::crypt_keyslot_add_by_volume_key(
                self.crypt_device,
                libcryptsetup::CRYPT_ANY_SLOT,
                ptr::null(),
                0,
                passphrase.as_ptr(),
                passphrase.count_bytes(),
            )
        };
        if result < 0 {
            return Err(Error::AddKeyslot);
        }

        Ok(())
    }

    // TODO: Requires admin permissions, what do?
    //
    /// Map crypt device.
    fn map(&mut self, passphrase: &CStr) -> Result<(), crate::Error> {
        // Create mapped device name.
        let mapped_name = format!("homesec-{}", Uuid::new_v4());
        let c_mapped_name = CString::new(mapped_name.as_bytes())?;

        let result = unsafe {
            libcryptsetup::crypt_activate_by_passphrase(
                self.crypt_device,
                c_mapped_name.as_ptr(),
                libcryptsetup::CRYPT_ANY_SLOT,
                passphrase.as_ptr(),
                passphrase.count_bytes(),
                0,
            )
        };
        if result < 0 {
            return Err(Error::Map.into());
        }

        self.mapped_name = Some((mapped_name, c_mapped_name));

        Ok(())
    }

    // TODO: Shelling out to mkfs.ext4 sucks, maybe a different FS is easier?
    //
    /// Create ext4 filesystem.
    fn mkfs_ext4(&self) -> Result<(), crate::Error> {
        let mapped_name = match &self.mapped_name {
            Some((mapped_name, _)) => mapped_name,
            None => return Err(Error::Unmapped.into()),
        };

        let mapper_path = PathBuf::from("/dev/mapper").join(mapped_name);
        let mut mkfs = Command::new("mkfs.ext4").arg("-q").arg(mapper_path).spawn()?;
        if !mkfs.wait()?.success() {
            return Err(Error::Mkfs.into());
        }

        Ok(())
    }
}

impl Drop for Crypt {
    fn drop(&mut self) {
        println!("DROPPING"); // TODO
        if self.crypt_device.is_null() {
            return;
        }

        // Unmount the device.
        if let Some(path) = self.mount_path.take() {
            let flags = UnmountFlags::FORCE | UnmountFlags::DETACH;
            if let Err(err) = mount::unmount(path, flags) {
                eprintln!("[ERROR] Unmount failed: {err}");
            }
        }

        // Unmap LUKS device.
        if let Some((_, mapped_name)) = self.mapped_name.take() {
            let result =
                unsafe { libcryptsetup::crypt_deactivate(self.crypt_device, mapped_name.as_ptr()) };
            if result < 0 {
                eprintln!("[ERROR] Crypt device deactivation failed");
            }
        }

        unsafe { libcryptsetup::crypt_free(self.crypt_device) };
    }
}

/// Cryptsetup error.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("crypt device initialization failed")]
    Init,
    #[error("LUKS header loading failed")]
    Load,
    #[error("LUKS header writing failed")]
    Format,
    #[error("keyslot addition failed")]
    AddKeyslot,
    #[error("crypt device mapping failed")]
    Map,
    #[error("crypt device unmapping failed")]
    Unmap,
    #[error("crypt device is unmapped")]
    Unmapped,
    #[error("Failed to create EXT4 filesystem")]
    Mkfs,
}
