#!/usr/bin/env bash
# Makes the Debian package and the archive of the command from a release build.
# Usage: packaging/linux.sh <version> <out folder>
set -euo pipefail
version=$1
out=$2
arch=$(dpkg --print-architecture)
root=$(mktemp -d)
# mktemp gives 700; dpkg would carry it over as the mode of the package's root.
chmod 755 "$root"
mkdir -p "$out"

install -Dm755 target/release/apeiron "$root/usr/bin/apeiron"
install -Dm755 target/release/apeiron-desktop "$root/usr/bin/apeiron-desktop"
install -Dm644 LICENSE-MIT "$root/usr/share/doc/apeiron/LICENSE-MIT"
install -Dm644 LICENSE-APACHE "$root/usr/share/doc/apeiron/LICENSE-APACHE"
python3 packaging/make_icons.py "$root/icons"
for size in 48 128 256 512; do
  install -Dm644 "$root/icons/icon-$size.png" \
    "$root/usr/share/icons/hicolor/${size}x${size}/apps/apeiron.png"
done
rm -rf "$root/icons"
install -Dm644 packaging/apeiron.desktop "$root/usr/share/applications/apeiron.desktop"

mkdir "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<EOF
Package: apeiron
Version: $version
Architecture: $arch
Maintainer: efe-clk <efe-clk@users.noreply.github.com>
Depends: libwebkit2gtk-4.1-0, libgtk-3-0
Section: utils
Priority: optional
Homepage: https://github.com/efe-clk/apeiron
Description: Back up, sync and keep versions of your files
 One program for backups, sync, versions and archives, with a desktop app.
EOF
dpkg-deb --root-owner-group --build "$root" "$out/apeiron_${version}_${arch}.deb"

# The command alone, for a server without a desktop.
tarball=$(mktemp -d)
mkdir "$tarball/apeiron-$version"
cp target/release/apeiron LICENSE-MIT LICENSE-APACHE "$tarball/apeiron-$version/"
tar -C "$tarball" -czf "$out/apeiron-$version-linux-$(uname -m).tar.gz" "apeiron-$version"
