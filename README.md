# Homesec

Homesec is a small Linux utility which uses user namespaces to isolate an
executable. It allows either discarding or encrypting all files an application
writes to the disk, while providing an escape hatch to manually persist files to
the real filesystem.

# Dependencies

To make use of the encrypted home directories, you must have [gocryptfs]
installed and in your `$PATH`.

[gocryptfs]: https://github.com/rfjakob/gocryptfs

# Usage

```text
Usage: homesec <cmd> [-e] [-i <id>] [args...]

Run applications with an isolated filesystem.

Positional Arguments:
  cmd               command which will be executed

Options:
  -e, --ephemeral   use tmpfs as home directory
  -i, --id          persistent storage identifier (default: command's name)
  --help            display usage information
```

Inside the read-only root filesystem, you can get access to the original
writable filesystem at `/tmp/write-root`.

To execute `bash` without persisting any data, run:

```sh
homesec -e bash
```

To share the same encrypted home between multiple applications, run:

```sh
homesec -i shared bash
homesec -i shared sh
```

The encrypted directories are in the format `<ID>.homesec` and can be found at
`${XDG_DATA_HOME:-$HOME/.local/share}/homesec`.
