#!/usr/bin/env python3
"""MiniMax 图片生成小工具（品牌概念探索用）。

认证复用本机已配置的 ANTHROPIC_AUTH_TOKEN / ANTHROPIC_BASE_URL（api.minimaxi.com），
不落盘任何凭证。

用法:
    python3 brand/tools/minimax_image.py <prompt> [--model image-01] [--aspect 1:1] [--out DIR]
"""

import argparse
import json
import os
import pathlib
import urllib.error
import urllib.request

DEFAULT_HOST = "https://api.minimaxi.com"


def api_key() -> str:
    key = os.environ.get("MINIMAX_API_KEY") or os.environ.get("ANTHROPIC_AUTH_TOKEN")
    if not key:
        raise SystemExit("找不到 MiniMax API key：设置 MINIMAX_API_KEY 或 ANTHROPIC_AUTH_TOKEN")
    return key


def host() -> str:
    base = os.environ.get("MINIMAX_HOST") or DEFAULT_HOST
    # ANTHROPIC_BASE_URL 形如 https://api.minimaxi.com，可能是 /anthropic 之类的后缀
    return base.rstrip("/")


def generate(prompt: str, model: str, aspect: str, n: int = 1) -> dict:
    body = {
        "model": model,
        "prompt": prompt,
        "aspect_ratio": aspect,
        "n": n,
        "response_format": "url",
        "prompt_optimizer": True,
    }
    req = urllib.request.Request(
        host() + "/v1/image_generation",
        data=json.dumps(body).encode(),
        headers={"Authorization": "Bearer " + api_key(), "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=300) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as exc:
        return {"__http_error__": exc.code, "body": exc.read().decode()[:1200]}
    except urllib.error.URLError as exc:
        return {"__url_error__": str(exc)}


def download(url: str, dest: pathlib.Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url, timeout=300) as resp:
        dest.write_bytes(resp.read())


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("prompt")
    ap.add_argument("--model", default="image-01")
    ap.add_argument("--aspect", default="1:1")
    ap.add_argument("--out", default="brand/candidates")
    ap.add_argument("--name", default=None)
    args = ap.parse_args()

    result = generate(args.prompt, args.model, args.aspect)
    print(json.dumps(result, ensure_ascii=False)[:2000])

    data = (result.get("data") or {}).get("image_urls") or []
    if not data:
        return
    out_dir = pathlib.Path(args.out)
    stem = args.name or "candidate"
    for index, url in enumerate(data, start=1):
        suffix = ".png" if url.lower().split("?")[0].endswith(".png") else ".jpg"
        target = out_dir / f"{stem}-{index}{suffix}"
        download(url, target)
        print("saved:", target)


if __name__ == "__main__":
    main()
