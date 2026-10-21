#!/bin/sh
# Installs Lumen Edit for the current user (no root needed):
#   Linux: ~/.local/bin/lumen-edit, agents in ~/.local/share/lumen-edit/agents,
#          a desktop entry;
#   macOS: ~/Applications/Lumen Edit.app and a ~/.local/bin/lumen-edit link;
#   both:  the MCP server in Claude Code (if installed).
set -e
cd "$(dirname "$0")/.."
PREFIX="${PREFIX:-$HOME/.local}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

cargo build --release
./scripts/build-agents.sh

register_mcp() {
    if command -v claude >/dev/null 2>&1; then
        claude mcp remove codium --scope user >/dev/null 2>&1 || true
        claude mcp remove lumen --scope user >/dev/null 2>&1 || true
        claude mcp add lumen --scope user -- "$PREFIX/bin/lumen-edit" mcp
    fi
}

if [ "$(uname -s)" = Darwin ]; then
    ./scripts/bundle-macos.sh target/release/lumen-edit target >/dev/null
    mkdir -p "$HOME/Applications" "$PREFIX/bin"
    rm -rf "$HOME/Applications/Lumen Edit.app"
    cp -R "target/Lumen Edit.app" "$HOME/Applications/"
    ln -sf "$HOME/Applications/Lumen Edit.app/Contents/MacOS/lumen-edit" "$PREFIX/bin/lumen-edit"
    register_mcp
    echo "Installed: $HOME/Applications/Lumen Edit.app, $PREFIX/bin/lumen-edit"
    exit 0
fi

# Agents named by arch only, from before macOS and Windows hosts.
rm -f "$DATA/lumen-edit/agents/lumen-agent-x86_64" "$DATA/lumen-edit/agents/lumen-agent-aarch64"
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

register_mcp

echo "Installed: $PREFIX/bin/lumen-edit"
