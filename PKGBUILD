# Maintainer: Jayson Lennon <jayson@jaysonlennon.dev>

pkgname=jinn
pkgver=0.89.1
pkgrel=1
pkgdesc='Agentic LLM agent harness'
url='https://github.com/jayson-lennon/jinn'
license=(AGPL-3.0)
makedepends=('cargo' 'clang')
checkdepends=('upx')
depends=('sqlite' 'gcc-libs')
arch=('x86_64' 'aarch64')

# Build from local checkout. Run makepkg from the project root directory.
# No source array — we reference $startdir directly.
options=(!debug)
source=()

prepare() {
    ln -sf "$startdir" "$srcdir/$pkgname-$pkgver"
    cd "$srcdir/$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$srcdir/$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    CFLAGS+=" -ffat-lto-objects" cargo build --frozen --release
    command -v upx >/dev/null 2>&1 && upx -9 target/release/jinn || echo 'upx not found, skipping binary compression'
}

package() {
    cd "$srcdir/$pkgname-$pkgver"

    # Install binary.
    install -Dm0755 target/release/jinn -t "$pkgdir/usr/bin/"

    # Install default themes, personas, and prompts to /usr/share/jinn/.
    install -Dm0644 -t "$pkgdir/usr/share/jinn/themes/" res/themes/*.toml
    install -Dm0644 -t "$pkgdir/usr/share/jinn/personas/" res/personas/*.md
    install -Dm0644 -t "$pkgdir/usr/share/jinn/prompts/" res/prompts/*.md

    # Install default plugins to /usr/share/jinn/plugins/, preserving the
    # global/attachable/meta split expected by discover_plugins. global and
    # attachable are nested (<kind>/<plugin>/init.lua); meta is flat (meta/*.lua).
    for kind in global attachable; do
        for plugin_dir in res/plugins/$kind/*/; do
            local plugin_name=$(basename "$plugin_dir")
            for file in "$plugin_dir"*; do
                install -Dm0644 "$file" -t "$pkgdir/usr/share/jinn/plugins/$kind/$plugin_name/"
            done
        done
    done
    install -Dm0644 -t "$pkgdir/usr/share/jinn/plugins/meta/" res/plugins/meta/*.lua

    # Install default skills to /usr/share/jinn/skills/.
    for skill_dir in res/skills/*/; do
        local skill_name=$(basename "$skill_dir")
        for file in "$skill_dir"*; do
            install -Dm0644 "$file" -t "$pkgdir/usr/share/jinn/skills/$skill_name/"
        done
    done

    # Install shell completions.
    local _bin="target/release/jinn"
    install -Dm0644 /dev/stdin "$pkgdir/usr/share/bash-completion/completions/jinn" \
        < <("$_bin" completions bash)
    install -Dm0644 /dev/stdin "$pkgdir/usr/share/zsh/site-functions/_jinn" \
        < <("$_bin" completions zsh)
    install -Dm0644 /dev/stdin "$pkgdir/usr/share/fish/vendor_completions.d/jinn.fish" \
        < <("$_bin" completions fish)
}
