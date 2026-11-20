#!/usr/bin/env bash
# Makes the RPM package (Fedora, RHEL, openSUSE) from a release build.
# Usage: packaging/rpm.sh <version> <out folder>
set -euo pipefail
version=$1
out=$(realpath -m "$2")
arch=$(uname -m)
work=$(mktemp -d)
root="$work/root"
mkdir -p "$out" "$work/SPECS" "$work/RPMS"

install -Dm755 target/release/apeiron "$root/usr/bin/apeiron"
install -Dm755 target/release/ape "$root/usr/bin/ape"
install -Dm755 target/release/apeiron-desktop "$root/usr/bin/apeiron-desktop"
install -Dm644 LICENSE-MIT "$root/usr/share/licenses/apeiron/LICENSE-MIT"
install -Dm644 LICENSE-APACHE "$root/usr/share/licenses/apeiron/LICENSE-APACHE"
python3 packaging/make_icons.py "$work/icons"
for size in 48 128 256 512; do
  install -Dm644 "$work/icons/icon-$size.png" \
    "$root/usr/share/icons/hicolor/${size}x${size}/apps/apeiron.png"
done
install -Dm644 packaging/apeiron.desktop "$root/usr/share/applications/apeiron.desktop"

cat > "$work/SPECS/apeiron.spec" <<SPEC
Name: apeiron
Version: $version
Release: 1
Summary: Back up, sync and keep versions of your files
License: MIT OR Apache-2.0
URL: https://github.com/efe-clk/apeiron
Requires: webkit2gtk4.1, gtk3
# The programs are built already; they are packed as they are.
%global debug_package %{nil}
%global __os_install_post %{nil}

%description
One program for backups, sync, versions and archives, with a desktop app.

%install
cp -a $root/. %{buildroot}/

%files
/usr/bin/apeiron
/usr/bin/ape
/usr/bin/apeiron-desktop
/usr/share/applications/apeiron.desktop
/usr/share/icons/hicolor/*/apps/apeiron.png
%license /usr/share/licenses/apeiron/LICENSE-MIT
%license /usr/share/licenses/apeiron/LICENSE-APACHE
SPEC

rpmbuild -bb --target "$arch" --define "_topdir $work" \
  --define "_rpmdir $work/RPMS" "$work/SPECS/apeiron.spec" > /dev/null
cp "$work"/RPMS/*/apeiron-"$version"-1.*.rpm "$out/"
