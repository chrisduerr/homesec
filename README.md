# Homesick

Homesick is a small Linux utility which uses user namespaces to isolate an
executable. As a result it prevents applications from writing random files to
the user's home directory, while still giving access to the persistent root.

# Usage

```text
# Run bash with homesick isolation:
homesick bash

# `ls` inside the isolated home directory will show nothing:
$ ls ~ | wc -l
0

# Using `/tmp/write-root`, things can still be persisted manually.
$ echo "demo" > /tmp/write-root/home/user/demo
```
