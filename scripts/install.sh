#!/bin/sh
# Installs Codium RS for the current user (no root needed):
#   ~/.local/bin/codium-rs, agents in ~/.local/share/codium-rs/agents,
#   a desktop entry, and the MCP server in Claude Code (if installed).
set -e
cd "$(dirname "$0")/.."
PREFIX="${PREFIX:-$HOME/.local}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

cargo build --release
./scripts/build-agents.sh

install -Dm755 target/release/codium-rs "$PREFIX/bin/codium-rs"
for agent in target/agents/codium-agent-*; do
    install -Dm755 "$agent" "$DATA/codium-rs/agents/$(basename "$agent")"
done
install -Dm644 assets/codium-rs.svg "$DATA/icons/hicolor/scalable/apps/codium-rs.svg"
mkdir -p "$DATA/applications"
cat > "$DATA/applications/codium-rs.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Codium RS
GenericName=Редактор кода
Comment=Нативный редактор: локально и по SSH, LSP, терминал, git
Exec=$PREFIX/bin/codium-rs %F
Icon=codium-rs
Terminal=false
Categories=Development;TextEditor;IDE;
MimeType=text/plain;inode/directory;
StartupWMClass=codium-rs
DESKTOP
update-desktop-database "$DATA/applications" 2>/dev/null || true

if command -v claude >/dev/null 2>&1; then
    claude mcp remove codium --scope user >/dev/null 2>&1 || true
    claude mcp add codium --scope user -- "$PREFIX/bin/codium-rs" mcp
fi

echo "Готово: $PREFIX/bin/codium-rs"
