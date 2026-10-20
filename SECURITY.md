# Security Policy

## Supported versions

Only the latest release receives security fixes.

## Reporting a vulnerability

Please do **not** open a public issue for security problems. Report them privately through
[GitHub security advisories](https://github.com/anatoliiii/lumen-edit-rs/security/advisories/new).
You will get a reply within a week.

## Security notes

- The control socket (`$XDG_RUNTIME_DIR/lumen-edit/ctl-<pid>.sock`) lets any process of the same user drive the editor, including running shell commands on the project host. `$XDG_RUNTIME_DIR` is private to the user.
- SSH uses your system OpenSSH and its configuration; the editor never stores passwords or keys.
- `lumen-agent` is uploaded to `~/.cache/lumen-edit/` on remote hosts and only talks over the SSH channel.
- The pi agent tab runs inside a bubblewrap sandbox when `bwrap` is available: writes are limited to the project, `~/.pi`, caches and `/tmp`.
