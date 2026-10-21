#!/bin/sh
# Builds lumen-agent for the remote platforms this machine can build for and
# puts them where the editor looks: target/agents/lumen-agent-<os>-<arch>[.exe].
#   Linux:   static musl builds for x86_64 and aarch64 (linked by rust-lld,
#            so they build on any OS);
#   macOS:   the above plus macos-x86_64 and macos-aarch64;
#   Windows: the above plus windows-x86_64 (run from Git Bash).
# Agents for the other OSes come from release builds made on those systems.
# Requires: rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
set -e
cd "$(dirname "$0")/.."
mkdir -p target/agents

build() { # <rust target> <agent file name>
    cargo build --release -p lumen-agent --target "$1"
    exe=""
    case "$1" in *windows*) exe=".exe" ;; esac
    cp "target/$1/release/lumen-agent$exe" "target/agents/$2"
    echo "target/agents/$2"
}

# Off Linux the Linux agents are a bonus: release builds take them from Linux.
for arch in x86_64 aarch64; do
    build "$arch-unknown-linux-musl" "lumen-agent-linux-$arch" || [ "$(uname -s)" != Linux ]
done
case "$(uname -s)" in
Darwin)
    for arch in x86_64 aarch64; do
        rustup target add "$arch-apple-darwin" >/dev/null 2>&1 || true
        build "$arch-apple-darwin" "lumen-agent-macos-$arch"
    done
    ;;
MINGW* | MSYS* | CYGWIN*)
    build x86_64-pc-windows-msvc lumen-agent-windows-x86_64.exe
    ;;
esac
