#!/usr/bin/env python3

import hashlib
import pathlib
import re
import sys


TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
)


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: update-homebrew-formula.py <version> <dist-dir>")

    version = sys.argv[1]
    dist = pathlib.Path(sys.argv[2])
    formula = pathlib.Path("Formula/else.rb")
    text = formula.read_text()

    text = re.sub(
        r'^  version "[^"]*"$',
        f'  version "{version}"',
        text,
        count=1,
        flags=re.MULTILINE,
    )

    lines = text.splitlines()
    for target in TARGETS:
        archive = dist / f"else-{target}.tar.gz"
        if not archive.is_file():
            raise SystemExit(f"missing release archive: {archive}")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()

        marker = f"/else-{target}.tar.gz"
        for index, line in enumerate(lines):
            if not line.lstrip().startswith("url ") or marker not in line:
                continue
            if index + 1 >= len(lines):
                raise SystemExit(f"missing sha256 after URL for {target}")
            lines[index + 1] = re.sub(
                r'sha256 "[^"]*"',
                f'sha256 "{digest}"',
                lines[index + 1],
                count=1,
            )
            break
        else:
            raise SystemExit(f"formula URL not found for {target}")

    formula.write_text("\n".join(lines) + "\n")
    print(f"updated {formula} for {version}")


if __name__ == "__main__":
    main()
