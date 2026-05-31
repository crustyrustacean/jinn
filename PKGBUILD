# Maintainer: Jayson Lennon <jayson@jaysonlennon.dev>

pkgname=jinn
pkgver=0.46.0
pkgrel=1
pkgdesc='Agentic LLM agent harness'
url='https://github.com/jayson-lennon/jinn'
license=(AGPL-3.0)
makedepends=('cargo' 'clang' 'upx')
depends=('sqlite' 'gcc-libs')
arch=('x86_64')

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
    upx -9 target/release/jinn
}

package() {
    cd "$srcdir/$pkgname-$pkgver"

    # Install binary.
    install -Dm0755 target/release/jinn -t "$pkgdir/usr/bin/"

    # Install default themes, personas, and prompts to /usr/share/jinn/.
    install -Dm0644 -t "$pkgdir/usr/share/jinn/themes/" res/themes/*.toml
    install -Dm0644 -t "$pkgdir/usr/share/jinn/personas/" res/personas/*.md
    install -Dm0644 -t "$pkgdir/usr/share/jinn/prompts/" res/prompts/*.md

    # Install default plugins to /usr/share/jinn/plugins/.
    for plugin_dir in res/plugins/*/; do
        local plugin_name=$(basename "$plugin_dir")
        install -Dm0644 "$plugin_dir"init.lua -t "$pkgdir/usr/share/jinn/plugins/$plugin_name/"
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
