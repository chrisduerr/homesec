//! Linux user namespace handling.

// Some of the code in this file is inspired by Birdcage:
// https://github.com/phylum-dev/birdcage/blob/main/src/linux/namespaces.rs

use std::path::{Path, PathBuf};
use std::{env, fs, io};

use rustix::thread::{self, Gid, Uid, UnshareFlags};

/// Change root directory to `new_root` and mount the old root in `put_old`.
///
/// The `put_old` directory must be at or underneath `new_root`.
pub fn pivot_root(new_root: impl AsRef<Path>, put_old: impl AsRef<Path>) -> io::Result<()> {
    // Get target working directory path.
    let working_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    // Move root to its new location.
    rustix::process::pivot_root(new_root.as_ref(), put_old.as_ref())?;

    // Attempt to recover working directory, or switch to root.
    //
    // Without this, the user's working directory would stay the same, giving him
    // full access to it even if it is not bound.
    env::set_current_dir(working_dir).or_else(|_| env::set_current_dir("/"))?;

    Ok(())
}

/// Create a new user namespace.
///
/// The parent and child UIDs and GIDs define the user and group mappings
/// between the parent namespace and the new user namespace.
pub fn create_user_namespace(
    child_uid: Uid,
    child_gid: Gid,
    extra_flags: UnshareFlags,
) -> io::Result<()> {
    // Get current user's EUID and EGID.
    let parent_euid = rustix::process::geteuid();
    let parent_egid = rustix::process::getegid();

    // Create the namespace.
    thread::unshare(UnshareFlags::NEWUSER | extra_flags)?;

    // Map the UID and GID.
    map_ids(parent_euid, parent_egid, child_uid, child_gid)?;

    Ok(())
}

/// Update /proc uid/gid maps.
///
/// This should be called after creating a user namespace to ensure proper ID
/// mappings.
fn map_ids(parent_euid: Uid, parent_egid: Gid, child_uid: Uid, child_gid: Gid) -> io::Result<()> {
    let uid_map = format!("{} {} 1\n", child_uid.as_raw(), parent_euid.as_raw());
    let gid_map = format!("{} {} 1\n", child_gid.as_raw(), parent_egid.as_raw());
    fs::write("/proc/self/uid_map", uid_map.as_bytes())?;
    fs::write("/proc/self/setgroups", b"deny")?;
    fs::write("/proc/self/gid_map", gid_map.as_bytes())?;
    Ok(())
}
