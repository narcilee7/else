#!/usr/bin/env python3
"""Rewrite the product version everywhere it is written down.

The version lives in more places than it should: every workspace crate's
`Cargo.toml` (package version and inter-crate dependency requirements), the
README's release commands, and the release workflow's default input. Doing it by
hand is how you end up with a release whose binary reports 0.1.3 while the
formula says 0.1.2.

Usage:
    scripts/bump-version.py <version>          # rewrite to <version>
    scripts/bump-version.py --next patch       # bump from the current version

Prints every file it touched. Exits non-zero if the requested version is not a
plain `MAJOR.MINOR.PATCH`.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
# 版本的唯一权威来源：CLI crate 的 package version。
SOURCE = ROOT / "surfaces" / "cli" / "Cargo.toml"
# 允许 beta / rc 后缀：README 里承诺过 `v0.2.0-beta.1` 这种标签。
SEMVER = re.compile(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$")


def current_version() -> str:
    text = SOURCE.read_text(encoding="utf-8")
    match = re.search(r'^version = "([^"]+)"', text, re.MULTILINE)
    if not match:
        raise SystemExit(f"cannot find a version in {SOURCE}")
    return match.group(1)


def next_version(current: str, part: str) -> str:
    # 预发布版本升级时丢掉后缀：从 0.2.0-beta.1 的 patch 是 0.2.0，
    # 不是一个 0.2.0-beta.2（预发布编号由人自己决定）。
    stable = current.split("-")[0]
    major, minor, patch = (int(piece) for piece in stable.split("."))
    if part == "major":
        return f"{major + 1}.0.0"
    if part == "minor":
        return f"{major}.{minor + 1}.0"
    if part == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise SystemExit(f"unknown bump part: {part}")


def targets(current: str) -> list[pathlib.Path]:
    """Every file that mentions the current version in a version-shaped way."""
    files: list[pathlib.Path] = []
    for path in sorted(ROOT.rglob("Cargo.toml")):
        # 只改这个 checkout。`target/` 是构建产物，`.delta/` 是 Delta 的
        # 嵌套工作树 —— 那是另一份 checkout，改它等于在背后改别人的工作区。
        if {"target", "node_modules", ".git"} & set(path.parts):
            continue
        relative = path.relative_to(ROOT)
        if relative.parts and relative.parts[0] == ".delta":
            continue
        if f'version = "{current}"' in path.read_text(encoding="utf-8"):
            files.append(path)
    for relative in ("README.md", ".github/workflows/release-cli.yml"):
        path = ROOT / relative
        if path.exists() and current in path.read_text(encoding="utf-8"):
            files.append(path)
    return files


def main() -> None:
    args = sys.argv[1:]
    current = current_version()
    if len(args) == 2 and args[0] == "--next":
        wanted = next_version(current, args[1])
    elif len(args) == 1:
        wanted = args[0]
    else:
        raise SystemExit("usage: bump-version.py <version> | --next patch|minor|major")

    if not SEMVER.match(wanted):
        raise SystemExit(f"not a MAJOR.MINOR.PATCH version: {wanted}")

    if wanted == current:
        print(f"already at {current}; nothing to rewrite")
        return

    touched = []
    for path in targets(current):
        text = path.read_text(encoding="utf-8")
        # 只替换版本位置，不碰版本号里恰好出现的其它数字串。
        updated = text.replace(f'version = "{current}"', f'version = "{wanted}"')
        updated = updated.replace(f"scripts/release.sh {current}", f"scripts/release.sh {wanted}")
        updated = updated.replace(
            f"-f version={current}", f"-f version={wanted}"
        )
        updated = updated.replace(f"default: {current}", f"default: {wanted}")
        if updated != text:
            path.write_text(updated, encoding="utf-8")
            touched.append(path.relative_to(ROOT))

    if not touched:
        raise SystemExit(f"no file mentioned {current}; refusing to pretend it bumped")
    print(f"{current} -> {wanted}")
    for path in touched:
        print(f"  {path}")


if __name__ == "__main__":
    main()
