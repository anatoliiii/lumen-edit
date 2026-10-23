# Pi integration review — 2026-10-23

Baseline: editor `0d5b4ae` (release/v0.3.0), installed Pi `@earendil-works/pi-coding-agent` 1.3.0. This is a review of the editor integration, not an inventory of everything installed in the user's Pi configuration.

Evidence: `src/app/agent.rs`, `src/app/control.rs`, `src/app/mod.rs`, `src/config.rs`, `src/control/methods.rs`; installed Pi's `docs/rpc-commands.md`, `docs/rpc-extension-ui.md`, `docs/sessions.md`, and `dist/core/session-manager.js`. Public protocol reference: https://github.com/earendil-works/pi/tree/main/packages/coding-agent/docs (the locally installed 1.3.0 documentation is authoritative for this review).

## Baseline functionality

| Capability | Editor before this change | Evidence / limitation |
| --- | --- | --- |
| Streaming chat, reasoning, tool output, errors | Available | JSONL reader, message/tool events, Markdown transcript |
| Steer a running agent, queue a follow-up, stop | Available, partial | Enter/Alt+Enter, abort; queue contents are invisible and cannot be individually managed |
| Models, thinking levels, extension modes | Available | Model picker, thinking combo, `/mode` from configured extension; extension modes are not core Pi features |
| Skills, prompt templates, extension commands | Partial | Loaded by Pi and callable through slash input; no resource browser, built-in TUI commands are not automatically RPC commands |
| Agent control through MCP | Broken for actions | Broker removed the native `action` parameter even for the agent method, so calls fell back to transcript |
| MCP editor tools and remote project access | Available | Local Pi subprocess; SSH work goes through editor MCP, local builtin tools disabled for SSH |
| Local sandbox | Available, conditional | bubblewrap when configured and installed; does not isolate simultaneous agents from each other's project writes |
| Confirmation, selection, input, editor dialogs | Available | Viewport-sized dialog after preceding hotfix; no timeout cleanup, no global indicator for hidden agents waiting for input |
| Extension status, widgets, notifications | Partial | Text widgets/status/notifications; setTitle ignored, widget height unbounded |
| Session history, search, rename current session | Partial | Popup lists up to 200 JSONL files; fixed width; brittle text-based metadata scan; clicking history replaces current session |
| Independent concurrent Pi sessions | Basic | Separate processes/tabs after hotfix; no global management, identity or role |
| Resume exact session on restart | Missing | Restart uses `--continue`, which can resume a different agent's most recent session |
| Restore open agent sessions when reopening project/editor | Missing | Only document tabs persist |
| Address a specific agent through CLI/MCP | Missing | All calls route through active/docked agent; no ID parameter/list of live agents |
| Coordinator/worker groups and task/result delivery | Missing | No group model, task dispatch, result aggregation or group stop |
| Session totals and context occupancy | Missing | Last response usage only; get_session_stats response ignored |
| Clone/fork, session tree, entries | Missing | Pi RPC supports these, editor has no dedicated controls |
| Export HTML, retry and auto-compaction controls | Missing | Pi RPC supports these; editor only exposes manual compact |
| Images in prompts/transcripts | Partial | Text file references and `[image]` placeholder; no binary image attachment UI |
| Custom terminal components, interactive terminal apps | Not available over RPC | Pi itself does not forward custom TUI components to RPC clients |

## Changes selected after review

1. A session manager with live agents and searchable saved history, clear state and queue indicators, open/focus/rename/stop/close/restart/clone actions and reversible history archiving.
2. Exact session identity, duplicate-open prevention, stable named sessions, per-project session restoration without replaying prompts.
3. Independent sessions and coordinator/worker groups. Named, explicit tasks; per-agent model; one-shot delivery of completed/error/aborted worker results to the group's coordinator; group stop disables automatic continuation. Same project files are shared, and group instructions require disjoint edit scopes.
4. CLI/MCP agent IDs, live list, create/focus/close/rename/dispatch/group-stop/result inspection. Coordinators can delegate via the editor tools; workers cannot recursively create groups through the advertised coordinator workflow.
5. Proper RPC state, queue and session statistics; dialog expiration; widget bounds; exact restart paths.

No claim is made that every Pi feature becomes available. Full tree navigation, arbitrary TUI components, binary image attachments and provider configuration remain separate work. Validation and final behavior are recorded below after implementation.

## Implemented after review

- `src/app/agent/manager.rs`: lifecycle, per-project restoration, ID-based CLI/MCP operations, exact restart and coordinator delivery.
- `src/app/agent/manager_ui.rs`: live sessions, group controls, model selection, task/result inspection, saved history and archive controls.
- `src/app/agent/team.rs`: serializable group roles, saved sessions/drafts and task lifecycle.
- The chat remains available in both dock and tabs. Opening saved history focuses an existing instance in this editor instead of opening the same file twice.
- Independent groups have separate conversations. Workers within a group use the same project filesystem; no git worktrees or automatic merging are created. The limit is 24 open agents and eight workers per group.
- Worker completion, failure and cancellation produce one report per task. Reports go to the group's coordinator, using a follow-up when it is busy. Group stop clears unsent startup prompts and Pi queues and pauses delivery until explicitly resumed. Closing a coordinator stops and detaches its workers into independent sessions.
- Startup waits for saved history before sending an explicitly assigned initial task. Restoring an editor never replays work or wakes a coordinator. Empty sessions without a Pi JSONL file retain their name and draft; missing previously persisted files are reported.
- Session totals/context usage and queued messages are visible. Queues can be cleared as a whole; individual queue editing, tree navigation, fork-from-message, export and provider configuration remain unavailable through dedicated editor controls.
- Request expiration applies to hidden agents as well as visible dialogs. Extension widgets have bounded scrollable height. A process-generation check discards late output from an old process after restart. Exit/transport failure clears transient queue and compaction state so the session remains restartable.
- The normal MCP/CLI path carries the calling agent ID. Delegation is limited to a coordinator's own group, and workers cannot recursively create agents through that path. This is routing policy, not a security boundary against arbitrary project shell code.
- Only the latest task/result is retained in the manager record; full conversation history remains in Pi's JSONL. Delivery messages are capped at 12,000 characters, with a pointer to the full task/transcript. Process transport errors remain visible; this is not a transactional distributed task queue.
- Archive/duplicate-open guards cover agents in the current editor window; unrelated standalone Pi processes and other editor windows do not participate in a shared session lock.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets -- -D warnings`
- `cargo test --locked --workspace`: 74 tests passed, one LSP integration test ignored.
- `python3 scripts/test-agent-sessions.py`: actual isolated editor GUI plus deterministic RPC fixture. Checks independent processes, duplicate prevention, exact restart and stale exit events, caller routing and nested-delegation rejection through an actual stdio MCP subprocess (both catalogs), worker completion/error/cancellation delivery, group stop/resume, rename, clone, hidden dialog expiration, close via the ordinary tab command, and saved-session restoration without prompt replay. Xvfb allocates an independent display; user configuration and real Pi authentication are not used.
- `LUMEN_TEST_REAL_PI=/path/to/pi python3 scripts/test-agent-sessions.py`: installed Pi 1.3.0, isolated temporary HOME/config/runtime. Verified independent sessions, state and statistics, rename, loading synthetic saved history, cloning, exact restart, empty and saved session restoration, and close. No prompts were sent to a provider.
- Inspected a rendered manager frame. Live multi-provider/model task execution and SSH group execution were not exercised; task/result protocol behavior was checked with the fixture.
