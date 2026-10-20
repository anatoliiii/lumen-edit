#!/bin/sh
# Builds static lumen-agent binaries for the remote architectures and puts
# them where the editor looks: target/agents/lumen-agent-<arch>.
# Requires: rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
set -e
cd "$(dirname "$0")/.."
mkdir -p target/agents
for arch in x86_64 aarch64; do
    triple="$arch-unknown-linux-musl"
    cargo build --release -p lumen-agent --target "$triple"
    cp "target/$triple/release/lumen-agent" "target/agents/lumen-agent-$arch"
    echo "target/agents/lumen-agent-$arch"
done
