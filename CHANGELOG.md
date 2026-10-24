# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.4.0] - 2026-10-24

### Added
- Semantic inline completion: automatic 180 ms debounce, OpenRouter Codestral using Pi credentials, LSP/tree-sitter context, multiline ghost text, Tab acceptance and atomic Undo; explicit per-project cloud consent, bounded requests/cache and diagnostics.
- Agent sessions now live inside the project workspace: session list and groups beside the selected chat, inline creation, searchable model selection and session actions.
- Pi session manager: independent agents and coordinator/worker groups, task dispatch and result delivery, per-agent models, live status, queues, session totals and context usage.
- Saved agent sessions and drafts restore per project without replaying tasks; history can be opened separately, cloned or reversibly archived.
- Addressable CLI/MCP agent IDs, group controls and delegation through the editor tools.


### Fixed
- Settings use responsive form rows, sidebar navigation on wide windows, readable language-server cards and localized AI status with expandable diagnostics. Pi model limits no longer collapse into letter-by-letter columns.
- Verify Node.js downloads with native SHA-256 on all platforms, without relying on PowerShell or external checksum commands.
- Automatic inline completion no longer burns quota by repeatedly cancelling requests while typing; minute limits retry automatically, request accounting excludes pre-HTTP cancellation/cache, and the original overly small defaults are migrated.
- Inline completion now anchors the current insertion scope and rejects syntax regressions or misplaced Rust match-arm fragments, including inside macro token trees.
- Creating a background worker preserves the user's selected session and document.
- MCP forwarding preserves the native agent action parameter, enabling agent management and delegation through Pi tools.
- Pi restarts resume the exact session, ignore stale process events and cancel unsent startup prompts when stopped; expired background extension dialogs no longer block the UI.
- Pi extension confirmation and input dialogs fit the viewport, with scrollable content and visible response buttons.
- New Pi sessions open independent agent tabs; moving an agent into an occupied dock preserves the previous agent in a tab.
- Agent actions and file attachments target the active agent tab when several sessions are open.

## [0.3.0] - 2026-10-23

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
- Configurable keyboard shortcuts, including copy and paste in the editor and terminal, with a searchable shortcuts window and key recording.

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

[Unreleased]: https://github.com/anatoliiii/lumen-edit/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/anatoliiii/lumen-edit/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/anatoliiii/lumen-edit/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/anatoliiii/lumen-edit/releases/tag/v0.2.0
