use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::{fs, io};

use argh::FromArgs;
use io::Write;
use rustix::mount::{self, MountFlags};
use rustix::termios::{self, LocalModes, OptionalActions};
use rustix::thread::{Gid, Uid, UnshareFlags};
use xdg::BaseDirectories;

use crate::gocryptfs::Crypt;

mod gocryptfs;
mod namespaces;

/// Read-write location of the root directory inside the namespace.
const WRITE_ROOT: &str = "/tmp/write-root";

/// Run applications with an isolated filesystem.
#[derive(FromArgs)]
struct Args {
    /// use tmpfs as home directory
    #[argh(switch, short = 'e')]
    ephemeral: bool,

    /// persistent storage identifier (default: command's name)
    #[argh(option, short = 'i')]
    id: Option<String>,

    /// command which will be executed
    #[argh(positional)]
    cmd: String,

    /// command arguments
    #[argh(positional, greedy)]
    args: Vec<String>,
}

fn main() {
    let args: Args = argh::from_env();

    // Error out with invalid configurations.
    if args.ephemeral && args.id.is_some() {
        eprintln!("Arguments --ephemeral and --id cannot be used together\n");
        eprintln!("Run {} --help for more information", env!("CARGO_PKG_NAME"));
        process::exit(1);
    }

    // Get gocryptfs storage directory.
    let crypt_dir = if args.ephemeral {
        None
    } else {
        let dirs = match BaseDirectories::with_prefix("homesec") {
            Ok(dirs) => dirs,
            Err(err) => {
                eprintln!("[ERROR] Failed to get XDG base directories: {err}");
                process::exit(2);
            },
        };
        let crypt_id = args.id.as_ref().unwrap_or(&args.cmd);
        Some(dirs.get_data_file(format!("{crypt_id}.homesec")))
    };

    // Create our target filesystem.
    if let Err(err) = readonly_root(crypt_dir.as_deref()) {
        eprintln!("[ERROR] Failed to create readonly root: {err}");
        process::exit(255);
    }

    // Launch user executable.
    let mut cmd = Command::new(args.cmd);
    for arg in args.args {
        cmd.arg(arg);
    }
    cmd.spawn().unwrap().wait().unwrap();
}

/// Switch to a readonly version of the filesystem.
///
/// The old root will be mounted in read-write mode at [`WRITE_ROOT`] inside the
/// new root, allowing manually persisting data to the filesystem.
fn readonly_root(crypt_dir: Option<&Path>) -> io::Result<()> {
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
    let write_root = join_absolute_paths("/tmp", WRITE_ROOT);
    fs::create_dir(&write_root)?;

    // Change new root to be readonly.
    mount::mount_remount("/tmp", MountFlags::BIND | MountFlags::RDONLY, "")?;

    // Switch to the new tmpfs filesystem root.
    namespaces::pivot_root("/tmp", &write_root)?;

    // Create fake home directory.
    if let Some(home) = home {
        create_home(&home, crypt_dir)?;
    }

    // Drop user namespace permissions.
    namespaces::create_user_namespace(euid, egid, UnshareFlags::empty())?;

    Ok(())
}

/// Create a fake home directory.
///
/// This will map a temporary directory over the user's home directory and do
/// just enough to ensure graphical applications are able to start.
fn create_home(home: &Path, crypt_dir: Option<&Path>) -> io::Result<()> {
    // Get home directory path inside write root.
    let write_home = join_absolute_paths(WRITE_ROOT, home);

    match crypt_dir {
        // Create encrypted home directory.
        Some(crypt_dir) => {
            let password = read_password()?;
            let write_crypt_dir = join_absolute_paths(WRITE_ROOT, crypt_dir);
            let mut crypt = Crypt::new(write_crypt_dir, &password)?;
            crypt.mount(home, &password)?;
        },
        // Create ephemeral home directory.
        None => mount::mount2(None::<&str>, home, Some("tmpfs"), MountFlags::empty(), None)?,
    }

    // Try to copy X.Org files.
    let _ = fs::copy(write_home.join(".Xauthority"), home.join(".Xauthority"));

    Ok(())
}

/// Combine two absolute paths.
///
/// This combines the `root` with `path` pretending that `path` starts with `./`
/// instead of `/`.
fn join_absolute_paths(root: impl Into<PathBuf>, path: impl AsRef<Path>) -> PathBuf {
    let mut joined = root.into();
    for segment in path.as_ref().iter().skip(1) {
        joined.push(segment);
    }
    joined
}

/// Read a password from STDIN.
fn read_password() -> io::Result<String> {
    // Prompt for password.
    print!("Password: ");
    io::stdout().flush()?;

    // Get current terminal config.
    let tty = File::open("/dev/tty")?;
    let mut termios = termios::tcgetattr(&tty)?;

    // Stop write-back of user input.
    termios.local_modes.remove(LocalModes::ECHO);
    termios.local_modes.insert(LocalModes::ECHONL);
    termios::tcsetattr(&tty, OptionalActions::Now, &termios)?;

    // Read the password.
    let reader = BufReader::new(&tty);
    let line =
        reader.lines().next().ok_or_else(|| io::Error::other("Failed to read password from STDIN"));

    // Reset terminal modes.
    termios.local_modes.remove(LocalModes::ECHONL);
    termios.local_modes.insert(LocalModes::ECHO);
    termios::tcsetattr(&tty, OptionalActions::Now, &termios)?;

    line?
}
