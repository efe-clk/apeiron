#!/usr/bin/env python3
"""Writes the package manifests of a published release (docs/GUIDE.md §2.4).

Usage: packaging/manifests.py <version> <out folder>

It reads the release's SHA256SUMS and the checksum of the source archive
GitHub makes for the tag, and writes:

  homebrew/apeiron.rb           the formula of the tap efe-clk/homebrew-apeiron
  aur/PKGBUILD, aur/.SRCINFO    the Arch User Repository package `apeiron`
  chocolatey/                   the Chocolatey package (apeiron.nuspec, tools/)
  winget/                       the manifests for microsoft/winget-pkgs

Publishing each one needs that service's account; see docs/GUIDE.md §2.4.
"""

import hashlib
import sys
import textwrap
import urllib.request
from pathlib import Path

REPO = "https://github.com/efe-clk/apeiron"
SUMMARY = "Backups, sync, versions and archives in one engine"
DESCRIPTION = (
    "Apeiron backs up, syncs, versions and archives folders with one engine: "
    "content-addressed, deduplicated and encrypted snapshots, a desktop app, "
    "a terminal interface and a web server for several users."
)


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "apeiron-release"})
    with urllib.request.urlopen(request, timeout=120) as answer:
        return answer.read()


def release_sums(version: str) -> dict[str, str]:
    text = fetch(f"{REPO}/releases/download/v{version}/SHA256SUMS").decode()
    sums = {}
    for line in text.splitlines():
        digest, name = line.split(maxsplit=1)
        sums[name.lstrip("*")] = digest
    return sums


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(text).lstrip())


def homebrew(out: Path, version: str, source: str) -> None:
    write(
        out / "homebrew/apeiron.rb",
        f'''
        class Apeiron < Formula
          desc "{SUMMARY}"
          homepage "{REPO}"
          url "{REPO}/archive/refs/tags/v{version}.tar.gz"
          sha256 "{source}"
          license any_of: ["MIT", "Apache-2.0"]

          depends_on "cmake" => :build
          depends_on "rust" => :build

          def install
            system "cargo", "install", *std_cargo_args(path: "crates/apeiron-cli")
          end

          test do
            assert_match version.to_s, shell_output("#{{bin}}/apeiron --version")
            system bin/"apeiron", "init", testpath/"repo"
            (testpath/"src/a.txt").write "hello"
            system bin/"ape", "--repo", testpath/"repo", "backup", testpath/"src"
            system bin/"apeiron", "--repo", testpath/"repo", "verify"
          end
        end
        ''',
    )


def aur(out: Path, version: str, source: str) -> None:
    write(
        out / "aur/PKGBUILD",
        f'''
        # Maintainer: efe-clk <efe-clk@users.noreply.github.com>
        pkgname=apeiron
        pkgver={version}
        pkgrel=1
        pkgdesc="{SUMMARY}"
        arch=('x86_64' 'aarch64')
        url="{REPO}"
        license=('MIT OR Apache-2.0')
        depends=('webkit2gtk-4.1' 'gtk3' 'gcc-libs' 'glibc')
        makedepends=('cargo' 'cmake' 'python')
        options=('!lto')
        source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
        sha256sums=('{source}')

        prepare() {{
          cd "$pkgname-$pkgver"
          export RUSTUP_TOOLCHAIN=stable
          cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
        }}

        build() {{
          cd "$pkgname-$pkgver"
          export RUSTUP_TOOLCHAIN=stable CARGO_TARGET_DIR=target
          cargo build --frozen --release -p apeiron-cli-rs -p apeiron-desktop-rs
        }}

        package() {{
          cd "$pkgname-$pkgver"
          install -Dm755 target/release/apeiron target/release/ape target/release/apeiron-desktop -t "$pkgdir/usr/bin/"
          install -Dm644 packaging/apeiron.desktop -t "$pkgdir/usr/share/applications/"
          python packaging/make_icons.py icons
          for size in 48 128 256 512; do
            install -Dm644 "icons/icon-$size.png" "$pkgdir/usr/share/icons/hicolor/${{size}}x${{size}}/apps/apeiron.png"
          done
          install -Dm644 LICENSE-MIT LICENSE-APACHE -t "$pkgdir/usr/share/licenses/$pkgname/"
        }}
        ''',
    )
    write(
        out / "aur/.SRCINFO",
        f'''
        pkgbase = apeiron
        \tpkgdesc = {SUMMARY}
        \tpkgver = {version}
        \tpkgrel = 1
        \turl = {REPO}
        \tarch = x86_64
        \tarch = aarch64
        \tlicense = MIT OR Apache-2.0
        \tmakedepends = cargo
        \tmakedepends = cmake
        \tmakedepends = python
        \tdepends = webkit2gtk-4.1
        \tdepends = gtk3
        \tdepends = gcc-libs
        \tdepends = glibc
        \toptions = !lto
        \tsource = apeiron-{version}.tar.gz::{REPO}/archive/refs/tags/v{version}.tar.gz
        \tsha256sums = {source}

        pkgname = apeiron
        ''',
    )


def chocolatey(out: Path, version: str, installer: str, digest: str) -> None:
    write(
        out / "chocolatey/apeiron.nuspec",
        f'''
        <?xml version="1.0" encoding="utf-8"?>
        <package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
          <metadata>
            <id>apeiron</id>
            <version>{version}</version>
            <title>Apeiron</title>
            <authors>efe-clk</authors>
            <projectUrl>{REPO}</projectUrl>
            <licenseUrl>{REPO}/blob/master/LICENSE-MIT</licenseUrl>
            <projectSourceUrl>{REPO}</projectSourceUrl>
            <releaseNotes>{REPO}/releases/tag/v{version}</releaseNotes>
            <tags>backup sync versioning archive encryption</tags>
            <summary>{SUMMARY}</summary>
            <description>{DESCRIPTION}</description>
          </metadata>
          <files>
            <file src="tools\\**" target="tools" />
          </files>
        </package>
        ''',
    )
    write(
        out / "chocolatey/tools/chocolateyinstall.ps1",
        f'''
        $ErrorActionPreference = 'Stop'
        Install-ChocolateyPackage -PackageName 'apeiron' -FileType 'exe' `
          -Url64bit '{REPO}/releases/download/v{version}/{installer}' `
          -Checksum64 '{digest}' -ChecksumType64 'sha256' `
          -SilentArgs '/S'
        ''',
    )
    write(
        out / "chocolatey/tools/chocolateyuninstall.ps1",
        """
        $ErrorActionPreference = 'Stop'
        $key = Get-UninstallRegistryKey -SoftwareName 'Apeiron'
        if ($key) {
          Uninstall-ChocolateyPackage -PackageName 'apeiron' -FileType 'exe' `
            -SilentArgs '/S' -File $key.UninstallString.Trim('"')
        }
        """,
    )


def winget(out: Path, version: str, installer: str, digest: str) -> None:
    folder = out / "winget"
    head = "# yaml-language-server: $schema=https://aka.ms/winget-manifest"
    write(
        folder / "efe-clk.Apeiron.yaml",
        f"""
        {head}.version.1.10.0.schema.json
        PackageIdentifier: efe-clk.Apeiron
        PackageVersion: {version}
        DefaultLocale: en-US
        ManifestType: version
        ManifestVersion: 1.10.0
        """,
    )
    write(
        folder / "efe-clk.Apeiron.installer.yaml",
        f"""
        {head}.installer.1.10.0.schema.json
        PackageIdentifier: efe-clk.Apeiron
        PackageVersion: {version}
        InstallerType: nullsoft
        Scope: machine
        Installers:
          - Architecture: x64
            InstallerUrl: {REPO}/releases/download/v{version}/{installer}
            InstallerSha256: {digest.upper()}
        ManifestType: installer
        ManifestVersion: 1.10.0
        """,
    )
    write(
        folder / "efe-clk.Apeiron.locale.en-US.yaml",
        f"""
        {head}.defaultLocale.1.10.0.schema.json
        PackageIdentifier: efe-clk.Apeiron
        PackageVersion: {version}
        PackageLocale: en-US
        Publisher: efe-clk
        PublisherUrl: https://github.com/efe-clk
        PackageName: Apeiron
        PackageUrl: {REPO}
        License: MIT OR Apache-2.0
        LicenseUrl: {REPO}/blob/master/LICENSE-MIT
        ShortDescription: {SUMMARY}
        Description: {DESCRIPTION}
        ReleaseNotesUrl: {REPO}/releases/tag/v{version}
        Tags:
          - backup
          - sync
          - encryption
        ManifestType: defaultLocale
        ManifestVersion: 1.10.0
        """,
    )


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    version, out = sys.argv[1], Path(sys.argv[2])
    sums = release_sums(version)
    source = hashlib.sha256(fetch(f"{REPO}/archive/refs/tags/v{version}.tar.gz")).hexdigest()
    installer = f"Apeiron-{version}-windows-x86_64.exe"
    if installer not in sums:
        sys.exit(f"{installer} is not in the SHA256SUMS of v{version}")
    homebrew(out, version, source)
    aur(out, version, source)
    chocolatey(out, version, installer, sums[installer])
    winget(out, version, installer, sums[installer])
    print(f"wrote the manifests of {version} into {out}")


if __name__ == "__main__":
    main()
