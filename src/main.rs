use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::{env, fs, io};

use rustix::mount::{self, MountFlags};
use rustix::thread::{Gid, Uid, UnshareFlags};

mod namespaces;

/// Read-write location of the root directory inside the namespace.
const WRITE_ROOT: &str = "/tmp/write-root";

fn main() {
    // Complain if no args were provided.
    let mut args = env::args().skip(1);
    let cmd = match args.next() {
        Some(cmd) => cmd,
        None => {
            eprintln!("USAGE: homesick [COMMAND]");
            process::exit(1);
        },
    };

    // Create our target filesystem.
    if let Err(err) = readonly_root() {
        eprintln!("Failed to create readonly root: {err}");
        process::exit(255);
    }

    // Launch user executable.
    let mut cmd = Command::new(cmd);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.spawn().unwrap().wait().unwrap();
}

/// Switch to a readonly version of the filesystem.
///
/// The old root will be mounted in read-write mode at [`WRITE_ROOT`] inside the
/// new root, allowing manually persisting data to the filesystem.
fn readonly_root() -> io::Result<()> {
    let home = home::home_dir();
    let euid = rustix::process::geteuid();
    let egid = rustix::process::getegid();

    // Create a new user namespace to acquire mounting permissions.
    namespaces::create_user_namespace(Uid::ROOT, Gid::ROOT, UnshareFlags::NEWNS)?;

    // Map an in-memory filesystem on top of the existing /tmp.
    mount::mount2(None::<&str>, "/tmp", Some("tmpfs"), MountFlags::empty(), None)?;

    // Bind mount old root to the new tmpfs.
    mount::mount_recursive_bind("/", "/tmp")?;

    // Create a new memory filesystem to shadow the real /tmp.
    mount::mount2(None::<&str>, "/tmp/tmp", Some("tmpfs"), MountFlags::empty(), None)?;

    // Create mount location for the old root directory.
    let mut write_root = PathBuf::from("/tmp");
    for segment in Path::new(WRITE_ROOT).iter().skip(1) {
        write_root.push(segment);
    }
    fs::create_dir(&write_root)?;

    // Change new root to be readonly.
    mount::mount_remount("/tmp", MountFlags::BIND | MountFlags::RDONLY, "")?;

    // Switch to the new tmpfs filesystem root.
    namespaces::pivot_root("/tmp", &write_root)?;

    // Create fake home directory.
    if let Some(home) = home {
        create_home(&home)?;
    }

    // Drop user namespace permissions.
    namespaces::create_user_namespace(euid, egid, UnshareFlags::empty())?;

    Ok(())
}

/// Create a fake home directory.
///
/// This will map a temporary directory over the user's home directory and do
/// just enough to ensure graphical applications are able to start.
fn create_home(path: &Path) -> io::Result<()> {
    // Map temporary directory over the target path.
    mount::mount2(None::<&str>, path, Some("tmpfs"), MountFlags::empty(), None)?;

    // Try to copy X.Org files.
    let mut old_home = PathBuf::from(WRITE_ROOT);
    for segment in path.iter().skip(1) {
        old_home.push(segment);
    }
    let _ = fs::copy(old_home.join(".Xauthority"), path.join(".Xauthority"));

    Ok(())
}
