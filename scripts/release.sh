#!/usr/bin/env bash
# 打一个 release：编译、签名（macOS）、打包、算校验和、更新 Homebrew formula。
#
#   scripts/release.sh 0.1.0
#
# 上传到 GitHub 需要 gh CLI；没装的话会在最后一步停下，其余产物照常生成。

set -euo pipefail

VERSION="${1:-}"
[ -n "$VERSION" ] || { echo "用法：scripts/release.sh <version>" >&2; exit 1; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DIST="$ROOT/dist"
rm -rf "$DIST"
mkdir -p "$DIST"

TARGETS=(
  "aarch64-apple-darwin"
  "x86_64-apple-darwin"
  "aarch64-unknown-linux-gnu"
  "x86_64-unknown-linux-gnu"
)

echo "==> 编译 release 二进制"
cargo build --release -p else-cli

echo "==> 打包"
for target in "${TARGETS[@]}"; do
  archive="else-$target.tar.gz"
  mkdir -p "$DIST/$target"
  cp "target/release/else" "$DIST/$target/else"

  if [[ "$target" == *apple-darwin ]]; then
    # 签名与公证在 CI 里做（需要 Developer ID 证书）；本地先做 ad-hoc 签名，
    # 至少保证二进制完整。
    codesign --force --sign - "$DIST/$target/else" 2>/dev/null || true
  fi

  tar -czf "$DIST/$archive" -C "$DIST/$target" else
  rm -rf "$DIST/$target"
  echo "    $archive"
done

echo "==> 校验和"
( cd "$DIST" && shasum -a 256 ./*.tar.gz > SHA256SUMS )
cat "$DIST/SHA256SUMS"

echo "==> 更新 Homebrew formula"
python3 - "$VERSION" "$DIST" <<'PY'
import hashlib, pathlib, sys

version, dist = sys.argv[1], pathlib.Path(sys.argv[2])
formula = pathlib.Path("Formula/else.rb")
text = formula.read_text()

targets = {
    "REPLACE_WITH_ARM64_SHA256": "aarch64-apple-darwin",
    "REPLACE_WITH_X86_64_SHA256": "x86_64-apple-darwin",
    "REPLACE_WITH_LINUX_ARM64_SHA256": "aarch64-unknown-linux-gnu",
    "REPLACE_WITH_LINUX_X86_64_SHA256": "x86_64-unknown-linux-gnu",
}
for placeholder, target in targets.items():
    digest = hashlib.sha256((dist / f"else-{target}.tar.gz").read_bytes()).hexdigest()
    text = text.replace(placeholder, digest)

text = text.replace('version "0.1.0"', f'version "{version}"')
formula.write_text(text)
print(f"    Formula/else.rb → {version}")
PY

if command -v gh >/dev/null 2>&1; then
  echo "==> 上传 GitHub release"
  gh release create "v$VERSION" "$DIST"/*.tar.gz "$DIST/SHA256SUMS" \
    --title "else $VERSION" \
    --generate-notes
else
  echo "==> 没找到 gh CLI，跳过上传。产物在 $DIST"
fi

echo "==> 完成：$DIST"
