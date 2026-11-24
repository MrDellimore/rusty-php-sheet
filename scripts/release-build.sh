#!/usr/bin/env bash
# Build the extension for the PHP on PATH, test it, and package it for PIE.
#
#   RELEASE_VERSION=0.1.0 scripts/release-build.sh
#
# Writes dist/php_rusty_sheet-{version}_php{X.Y}-{arch}-{os}-{libc}-{nts|zts}.zip
# with rusty_sheet.so inside: the asset name PIE's `pre-packaged-binary` download
# method looks for (https://github.com/php/pie, docs/extension-maintainers.md).
# Used by .github/workflows/release.yml on Linux (glibc and, inside Alpine,
# musl) and macOS; runs locally too.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VERSION="${RELEASE_VERSION:?set RELEASE_VERSION, e.g. 0.1.0}"
VERSION="${VERSION#v}"
PHP="${PHP:-php}"

php_version=$("$PHP" -r 'echo PHP_MAJOR_VERSION.".".PHP_MINOR_VERSION;')
ts=$("$PHP" -r 'echo PHP_ZTS ? "zts" : "nts";')
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) echo "unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
case "$(uname -s)" in
    Linux)
        os=linux
        # Not `ldd --version | grep musl`: musl's ldd exits 1, which pipefail turns into
        # a failed test, so Alpine would be mislabelled glibc.
        if compgen -G '/lib/ld-musl-*' >/dev/null; then libc=musl; else libc=glibc; fi
        built=target/release/librusty_sheet.so
        ;;
    Darwin)
        os=darwin
        libc=bsdlibc
        built=target/release/librusty_sheet.dylib
        ;;
    *) echo "unsupported OS $(uname -s)" >&2; exit 1 ;;
esac

if [ "$libc" = musl ]; then
    # A cdylib can't be built against a static musl C runtime.
    export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=-crt-static"
fi

echo "==> build for PHP $php_version $ts on $arch-$os-$libc"
cargo build --release --locked -p rusty_sheet

echo "==> test the built extension"
(cd php && composer install --no-interaction --no-progress ${COMPOSER_FLAGS:-})
(cd php && "$PHP" -d "extension=$ROOT/$built" vendor/bin/pest --colors=never)

name="php_rusty_sheet-${VERSION}_php${php_version}-${arch}-${os}-${libc}-${ts}"
mkdir -p dist "dist/$name"
cp "$built" "dist/$name/rusty_sheet.so"
(cd "dist/$name" && zip -q "../$name.zip" rusty_sheet.so)
rm -rf "dist/$name"
echo "==> dist/$name.zip"
