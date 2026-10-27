# Maintainer: EduMelo <dev@edumelo.com.br>
pkgname=flamenote
pkgver=0.1.0
pkgrel=1
pkgdesc='Notas flutuantes a partir do texto selecionado, no espírito do Flameshot'
arch=('x86_64')
url='https://github.com/EduMelo/flamenote'
license=('MIT')
depends=('gtk3' 'libxkbcommon')
makedepends=('rust')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('5feb0a69688eac35f7f6106562a8c2d2c164cf24082d07ac536ef28e5723c8ad')

prepare() {
  cd "$pkgname-$pkgver"
  export CARGO_HOME="$srcdir/cargo-home"
  cargo fetch --locked
}

build() {
  cd "$pkgname-$pkgver"
  export CARGO_HOME="$srcdir/cargo-home"
  cargo build --release --locked --frozen
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
