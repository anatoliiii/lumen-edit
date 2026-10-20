#!/bin/sh
# Installs Lumen Edit for the current user (no root needed):
#   ~/.local/bin/lumen-edit, agents in ~/.local/share/lumen-edit/agents,
#   a desktop entry, and the MCP server in Claude Code (if installed).
set -e
cd "$(dirname "$0")/.."
PREFIX="${PREFIX:-$HOME/.local}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

cargo build --release
./scripts/build-agents.sh

install -Dm755 target/release/lumen-edit "$PREFIX/bin/lumen-edit"
for agent in target/agents/lumen-agent-*; do
    install -Dm755 "$agent" "$DATA/lumen-edit/agents/$(basename "$agent")"
done
install -Dm644 assets/lumen-edit.svg "$DATA/icons/hicolor/scalable/apps/lumen-edit.svg"
mkdir -p "$DATA/applications"
cat > "$DATA/applications/lumen-edit.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Lumen Edit
GenericName=Code Editor
GenericName[ru]=Редактор кода
Comment=Native editor: local and over SSH, LSP, terminal, git
Comment[ru]=Нативный редактор: локально и по SSH, LSP, терминал, git
Exec=$PREFIX/bin/lumen-edit %F
Icon=lumen-edit
Terminal=false
Categories=Development;TextEditor;IDE;
MimeType=text/plain;inode/directory;
StartupWMClass=lumen-edit
DESKTOP
update-desktop-database "$DATA/applications" 2>/dev/null || true

# Leftovers of the former name (Codium RS).
rm -f "$PREFIX/bin/codium-rs" "$DATA/applications/codium-rs.desktop" "$DATA/icons/hicolor/scalable/apps/codium-rs.svg"
rm -rf "$DATA/codium-rs"

if command -v claude >/dev/null 2>&1; then
    claude mcp remove codium --scope user >/dev/null 2>&1 || true
    claude mcp remove lumen --scope user >/dev/null 2>&1 || true
    claude mcp add lumen --scope user -- "$PREFIX/bin/lumen-edit" mcp
fi

echo "Installed: $PREFIX/bin/lumen-edit"
