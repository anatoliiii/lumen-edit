# Contributing

Thanks for your interest in Lumen Edit! Issues and pull requests are welcome.

## Getting started

```sh
git clone https://github.com/anatoliiii/lumen-edit-rs.git
cd lumen-edit-rs
cargo run -- .
```

Linux build dependencies (Debian/Ubuntu): `libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev` and a C compiler for the tree-sitter grammars.

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace   # the SSH test uses the built lumen-agent
cargo test --workspace
```

CI runs the same checks.

- Keep changes focused; one feature or fix per pull request.
- Match the surrounding code: naming, comment density, error messages in Russian for the UI.
- Add tests for logic that can be tested without a window (buffer, git parsing, protocol, CLI parsing).
- Every non-ASCII icon used in the UI must exist in the bundled fonts; the `icon_glyphs_exist` test checks this.
- New user-facing commands go into `src/app/commands.rs`; new scriptable operations go into the method registry in `src/control/methods.rs` so the CLI and MCP pick them up automatically.
- Update `CHANGELOG.md` under *Unreleased*.

## Trying remote mode without an SSH server

```sh
PATH=$PWD/scripts/fake-ssh-dir:$PATH cargo run -- ssh://fake$PWD
```

## Interface text and translations

Text in the code is English and goes through `t!("…")` (with placeholders: `t!("Saved {name}", name = path)`); longer texts live in `locales/en/` and `locales/ru/` and are read with `t_file!`. Add the Russian version of every new string to `locales/ru.toml` — `cargo test` fails if a string has no translation, if Cyrillic text is left in the code, or if a translation is no longer used.

## Commit messages

Short imperative subject line (English or Russian), optional body explaining *why*.
