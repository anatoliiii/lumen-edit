#!/bin/sh
# Builds static codium-agent binaries for the remote architectures and puts
# them where the editor looks: target/agents/codium-agent-<arch>.
# Requires: rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
set -e
cd "$(dirname "$0")/.."
mkdir -p target/agents
for arch in x86_64 aarch64; do
    triple="$arch-unknown-linux-musl"
    cargo build --release -p codium-agent --target "$triple"
    cp "target/$triple/release/codium-agent" "target/agents/codium-agent-$arch"
    echo "target/agents/codium-agent-$arch"
done
