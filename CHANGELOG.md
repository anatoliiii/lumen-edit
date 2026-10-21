# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- English and Russian interface: text in the code is English, Russian comes from `locales/`; follows the system locale or Settings → Language. A test keeps the catalog complete.
- Server pulse: live CPU, memory, disks and top processes of the project host, status bar indicator, charts panel, disk/memory alerts, `pulse` in CLI/MCP.
- Teleport: two-pane file transfer between local and SSH hosts (including server to server) with progress and cancel, desktop drag-and-drop upload, `transfer` in CLI/MCP.
- CI/CD awareness: platform detection and icons, pipeline-key, job-name, expression and variable highlighting for GitLab, GitHub, Gitea, Forgejo, Woodpecker, Drone and more.
- Highlighting for YAML, Makefile, Groovy (Jenkinsfile), HCL (Terraform) and Dockerfile.
- Markdown files open as a rendered preview (Ctrl+Alt+P toggles).
- Image viewer for PNG, JPEG, GIF, WebP, BMP and ICO with zoom and a transparency checkerboard; local images render inside Markdown.
- SVG files open as a live preview that follows edits; CSV/TSV files open as a table (delimiter detected, quoted fields supported).
- pi agent: dock beside the editor, "Ask π" on a selection, send files and fragments, clickable file links and commands from the agent.

- SSH health check: the host badge turns red when the connection drops; its menu shows connection and agent status, reconnects and restarts the remote agent. A dead agent is restarted automatically once the host answers.
- Panics are written to `~/.local/state/lumen-edit/crash.log`.
- macOS: fonts, `.app` bundle (`scripts/bundle-macos.sh`, `install.sh`), server pulse from sysctl/vm_stat/top; CI and releases for macOS and Windows.
- SSH between any OSes: the host's OS is detected and the agent is picked as `lumen-agent-<os>-<arch>`; release archives carry agents for Linux, macOS and Windows. Windows hosts need Git Bash as the OpenSSH shell.
- Branches checked out in another worktree are marked; switching to one offers to open that worktree.

### Fixed
- Images replaced on disk are shown anew in viewers and Markdown previews (textures were cached by path).
- HTML in Markdown (`<img>`, `<br>`, `<div>` …) renders instead of showing the tags.
- Tabs of files with the same name show their folder (`about.png · ru`).
- Deleting a selection that spans lines no longer crashes the editor.
- Opening a big remote folder (e.g. `/DATA`) no longer stalls: file watching is set up in the background, skips ignored/build folders and is capped, so the agent answers at once and exits when the editor disconnects.
- The pi agent is restarted for the new project when switching projects (it kept working on the old local folder).
- The host badge menu in the status bar opens again.
- An unknown CLI word (`lumen-edit foo`) prints an error instead of opening a window.

### Changed
- Renamed the project from Codium RS to Lumen Edit (`lumen-edit`); settings migrate automatically.

## [0.2.0] - 2026-10-21

First public version: a complete rewrite of the early prototype.

### Added
- Custom editor widget on a rope buffer: multiple cursors, undo/redo, smart indent, find and replace.
- tree-sitter highlighting for Rust, Bash, Python, C, C++, C#, Go, HTML, CSS, JavaScript, TypeScript, JSON and TOML.
- LSP client with completion, hover, signature help, definition, references, rename, code actions, formatting and diagnostics; native servers only by default.
- Remote projects over SSH through system OpenSSH with ControlMaster, plus the optional static `lumen-agent` (x86_64 and aarch64).
- Integrated terminal (PTY + vt100) with clickable `file:line:col`.
- Git: status, hunk staging, commit/amend, push/pull/sync, branches, remotes, stash, history graph, blame, conflict resolution.
- Task detection and `.lumen/tasks.toml`.
- Command palette, quick open, project search and replace.
- Control socket with CLI (`lumen-edit <method>`) and MCP server (`lumen-edit mcp`).
- pi coding agent tab with sessions, model/mode pickers and bubblewrap sandbox.
- About window, settings window, install script and desktop entry.

[Unreleased]: https://github.com/anatoliiii/lumen-edit-rs/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/anatoliiii/lumen-edit-rs/releases/tag/v0.2.0
