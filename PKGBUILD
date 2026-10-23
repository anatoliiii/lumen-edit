# Maintainer: Анатолий Копейкин <anatoliiii@users.noreply.github.com>
# Contributor: lumen-edit contributors
pkgname=${pkgname:-lumen-edit}
pkgver=0.3.0
pkgrel=1
arch=('x86_64' 'aarch64')
url='https://github.com/anatoliiii/lumen-edit'
license=('MIT')
install=('${pkgname}.install')
depends=('libxcb' 'libxkbcommon' 'wayland' 'pkg-config')
optdepends=('rust: to build from source
            xdg-user-dirs: for $XDG_CONFIG_HOME directories (Desktop, Documents)
            fonts-noto: for glyph coverage')
makedepends=('base-devel' 'pkg-config')
short_desc='Native Rust code editor with SSH remote, LSP, terminal and git'
pkgdesc='Native Rust code editor with SSH remote, LSP, terminal and git'
options=('!touchlibfont')

build() {
    cd "$srcdir/$pkgname"
    cargo build --release
}

package() {
    cd "$srcdir/$pkgname"
    install -Dm755 target/release/lumen-edit \
        "$pkgdir/opt/$pkgname/bin/lumen-edit"

    # Desktop entry and icon (XDG-compliant locations).
    install -Dm644 assets/lumen-edit.svg \
        "$pkgdir/usr/share/icons/hicolor/scalable/apps/lumen-edit.svg"
    mkdir -p "$pkgdir/usr/share/applications"
    cat > "$pkgdir/usr/share/applications/lumen-edit.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Lumen Edit
GenericName=Code Editor
Comment=Native editor: local and over SSH, LSP, terminal, git
Exec=/opt/$pkgname/bin/lumen-edit %F
Icon=lumen-edit
Terminal=false
Categories=Development;TextEditor;IDE;
MimeType=text/plain;inode/directory;
StartupWMClass=lumen-edit
DESKTOP
    update-desktop-database "$pkgdir/usr/share/applications"

    # Data directory for agents and cache.
    install -Dm755 "$srcdir/$pkgname/scripts/build-agents.sh" \
        "$pkgdir/opt/$pkgname/bin/build-agents.sh"
    mkdir -p "$pkgdir/usr/share/lumen-edit/agents"
}

sha256sums=('SKIP')
