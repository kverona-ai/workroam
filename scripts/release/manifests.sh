#!/usr/bin/env bash
# 根据产物目录里的文件与校验和，生成包管理器清单。
#
# 用法：manifests.sh <版本> <产物目录>
#
# 产物（写入产物目录）：
#   workroam.rb                    Homebrew formula（命令行 workroam + workroamd）
#   workroam-desktop.rb            Homebrew cask（桌面应用）
#   workroam.json                  Scoop 清单（命令行）
#   winget-manifests-<版本>.zip    winget 清单（桌面应用安装程序）
# 某个清单依赖的产物不存在时，跳过该清单。
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "用法：$0 <版本> <产物目录>" >&2
  exit 2
fi

version="$1"; dist="$2"
repo="${GITHUB_REPOSITORY:-kverona-ai/workroam}"
owner="${repo%%/*}"
homepage="https://github.com/$repo"
base="$homepage/releases/download/v$version"
desc_en="Hand off your AI agent workspace between machines"
desc_zh="带着 AI Agent 工作区，在每台电脑之间漫游"

sha() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

have() {
  for f in "$@"; do [ -f "$dist/$f" ] || return 1; done
}

c="workroam-$version"
d="Workroam-$version"

# Homebrew formula
if have "$c-aarch64-apple-darwin.tar.gz" "$c-x86_64-apple-darwin.tar.gz" \
        "$c-aarch64-unknown-linux-gnu.tar.gz" "$c-x86_64-unknown-linux-gnu.tar.gz"; then
  cat > "$dist/workroam.rb" <<RUBY
class Workroam < Formula
  desc "$desc_en"
  homepage "$homepage"
  version "$version"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "$base/$c-aarch64-apple-darwin.tar.gz"
      sha256 "$(sha "$dist/$c-aarch64-apple-darwin.tar.gz")"
    end
    on_intel do
      url "$base/$c-x86_64-apple-darwin.tar.gz"
      sha256 "$(sha "$dist/$c-x86_64-apple-darwin.tar.gz")"
    end
  end

  on_linux do
    on_arm do
      url "$base/$c-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "$(sha "$dist/$c-aarch64-unknown-linux-gnu.tar.gz")"
    end
    on_intel do
      url "$base/$c-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "$(sha "$dist/$c-x86_64-unknown-linux-gnu.tar.gz")"
    end
  end

  depends_on "git"

  def install
    bin.install "workroam", "workroamd"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/workroam --version")
  end
end
RUBY
  echo "$dist/workroam.rb"
fi

# Homebrew cask
if have "$d-macos-arm64.dmg" "$d-macos-x64.dmg"; then
  cat > "$dist/workroam-desktop.rb" <<RUBY
cask "workroam-desktop" do
  arch arm: "arm64", intel: "x64"

  version "$version"
  sha256 arm:   "$(sha "$dist/$d-macos-arm64.dmg")",
         intel: "$(sha "$dist/$d-macos-x64.dmg")"

  url "$homepage/releases/download/v#{version}/Workroam-#{version}-macos-#{arch}.dmg"
  name "Workroam"
  desc "$desc_en"
  homepage "$homepage"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Workroam.app"

  zap trash: [
    "~/Library/Application Support/Workroam",
    "~/Library/Caches/dev.workroam.desktop",
    "~/Library/Preferences/dev.workroam.desktop.plist",
  ]
end
RUBY
  echo "$dist/workroam-desktop.rb"
fi

# Scoop
if have "$c-x86_64-pc-windows-msvc.zip" "$c-aarch64-pc-windows-msvc.zip"; then
  cat > "$dist/workroam.json" <<JSON
{
  "version": "$version",
  "description": "$desc_en",
  "homepage": "$homepage",
  "license": "Apache-2.0",
  "depends": "git",
  "architecture": {
    "64bit": {
      "url": "$base/$c-x86_64-pc-windows-msvc.zip",
      "hash": "$(sha "$dist/$c-x86_64-pc-windows-msvc.zip")",
      "extract_dir": "$c-x86_64-pc-windows-msvc"
    },
    "arm64": {
      "url": "$base/$c-aarch64-pc-windows-msvc.zip",
      "hash": "$(sha "$dist/$c-aarch64-pc-windows-msvc.zip")",
      "extract_dir": "$c-aarch64-pc-windows-msvc"
    }
  },
  "bin": ["workroam.exe", "workroamd.exe"],
  "checkver": "github",
  "autoupdate": {
    "architecture": {
      "64bit": {
        "url": "$homepage/releases/download/v\$version/workroam-\$version-x86_64-pc-windows-msvc.zip",
        "extract_dir": "workroam-\$version-x86_64-pc-windows-msvc"
      },
      "arm64": {
        "url": "$homepage/releases/download/v\$version/workroam-\$version-aarch64-pc-windows-msvc.zip",
        "extract_dir": "workroam-\$version-aarch64-pc-windows-msvc"
      }
    },
    "hash": {
      "url": "\$baseurl/SHA256SUMS"
    }
  }
}
JSON
  echo "$dist/workroam.json"
fi

# winget
if have "$d-windows-x64-setup.exe" "$d-windows-arm64-setup.exe"; then
  id="Kverona.Workroam"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  dir="$work/manifests/k/Kverona/Workroam/$version"
  mkdir -p "$dir"
  schema="1.6.0"

  cat > "$dir/$id.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.$schema.schema.json
PackageIdentifier: $id
PackageVersion: $version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: $schema
YAML

  {
    cat <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.$schema.schema.json
PackageIdentifier: $id
PackageVersion: $version
InstallerLocale: en-US
MinimumOSVersion: 10.0.17763.0
Installers:
- Architecture: x64
  InstallerType: nullsoft
  InstallerUrl: $base/$d-windows-x64-setup.exe
  InstallerSha256: $(sha "$dist/$d-windows-x64-setup.exe" | tr 'a-f' 'A-F')
- Architecture: arm64
  InstallerType: nullsoft
  InstallerUrl: $base/$d-windows-arm64-setup.exe
  InstallerSha256: $(sha "$dist/$d-windows-arm64-setup.exe" | tr 'a-f' 'A-F')
YAML
    if have "$d-windows-x64.msi"; then
      cat <<YAML
- Architecture: x64
  InstallerType: wix
  InstallerUrl: $base/$d-windows-x64.msi
  InstallerSha256: $(sha "$dist/$d-windows-x64.msi" | tr 'a-f' 'A-F')
YAML
    fi
    cat <<YAML
ManifestType: installer
ManifestVersion: $schema
YAML
  } > "$dir/$id.installer.yaml"

  cat > "$dir/$id.locale.en-US.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.$schema.schema.json
PackageIdentifier: $id
PackageVersion: $version
PackageLocale: en-US
Publisher: $owner
PublisherUrl: https://github.com/$owner
PackageName: Workroam
PackageUrl: $homepage
License: Apache-2.0
LicenseUrl: $homepage/blob/v$version/LICENSE
ShortDescription: $desc_en
Moniker: workroam
Tags:
- ai-agent
- sync
- s3
- self-hosted
ReleaseNotesUrl: $homepage/releases/tag/v$version
ManifestType: defaultLocale
ManifestVersion: $schema
YAML

  cat > "$dir/$id.locale.zh-CN.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.locale.$schema.schema.json
PackageIdentifier: $id
PackageVersion: $version
PackageLocale: zh-CN
Publisher: $owner
PackageName: Workroam
ShortDescription: $desc_zh
ManifestType: locale
ManifestVersion: $schema
YAML

  dist_abs="$(cd "$dist" && pwd)"
  rm -f "$dist_abs/winget-manifests-$version.zip"
  (cd "$work" && zip -qr "$dist_abs/winget-manifests-$version.zip" manifests)
  echo "$dist/winget-manifests-$version.zip"
fi
