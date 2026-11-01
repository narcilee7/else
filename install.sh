#!/bin/sh
# else 一键安装。
#
#   curl -fsSL https://raw.githubusercontent.com/narcilee7/else/master/install.sh | sh
#
# 装完会：放好 `else` 二进制、准备 runtime 目录、注册后台常驻服务，
# 然后跑一次 `else doctor` 告诉你现在的状态。

set -eu

REPO="${ELSE_REPO:-narcilee7/else}"
VERSION="${ELSE_VERSION:-latest}"
INSTALL_DIR="${ELSE_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '\033[36m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[33m!!\033[0m %s\n' "$1" >&2; }
die() { printf '\033[31mxx\033[0m %s\n' "$1" >&2; exit 1; }

# —— 平台 ——
os="$(uname -s)"
case "$os" in
  Darwin) os_target="apple-darwin" ;;
  Linux) os_target="unknown-linux-gnu" ;;
  *) die "不支持的系统：$os" ;;
esac

arch="$(uname -m)"
case "$arch" in
  arm64 | aarch64) arch_target="aarch64" ;;
  x86_64 | amd64) arch_target="x86_64" ;;
  *) die "不支持的架构：$arch" ;;
esac

target="$arch_target-unknown-linux-gnu"
if [ "$os" = "Darwin" ]; then
  target="$arch_target-apple-darwin"
fi

# —— 版本 ——
if [ "$VERSION" = "latest" ]; then
  say "查找最新版本…"
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$VERSION" ] || die "找不到最新版本，请用 ELSE_VERSION 指定"
fi

asset="else-$target.tar.gz"
url="https://github.com/$REPO/releases/download/v$VERSION/$asset"
checksums_url="https://github.com/$REPO/releases/download/v$VERSION/SHA256SUMS"

say "下载 else ${VERSION} (${target})…"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
curl -fsSL "$url" -o "$tmp/$asset" || die "下载失败：$url"
curl -fsSL "$checksums_url" -o "$tmp/SHA256SUMS" || die "下载校验和失败：$checksums_url"

expected="$(awk -v asset="$asset" '$2 == asset { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || die "SHA256SUMS 中没有 $asset"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$asset" | awk '{ print $1 }')"
else
  actual="$(shasum -a 256 "$tmp/$asset" | awk '{ print $1 }')"
fi
[ "$actual" = "$expected" ] || die "$asset 校验失败"

say "解压…"
tar -xzf "$tmp/$asset" -C "$tmp"

# —— 安装 ——
mkdir -p "$INSTALL_DIR"
if [ -w "$INSTALL_DIR" ]; then
  install -m 0755 "$tmp/else" "$INSTALL_DIR/else"
else
  say "需要写入权限，用 sudo 安装到 $INSTALL_DIR"
  sudo install -m 0755 "$tmp/else" "$INSTALL_DIR/else"
fi
say "已安装：$INSTALL_DIR/else"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) warn "$INSTALL_DIR 不在 PATH 里。把它加进 shell 配置："
     warn "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

# —— 初始化 ——
if [ -x "$INSTALL_DIR/else" ]; then
  "$INSTALL_DIR/else" onboard || warn "初始化没完全成功，先看看 else doctor 的输出"
  echo
  "$INSTALL_DIR/else" doctor || true
fi

cat <<'EOF'

装好了。

  else            打开终端界面
  else brief      看今天要你管的事
  else doctor     检查运行状态

EOF
