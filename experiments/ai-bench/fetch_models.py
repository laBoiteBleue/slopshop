"""Download the benchmark's models, pinned and checked (experiment, not part of SlopShop).

Every file of models.tsv is fetched from Hugging Face at its pinned revision into the cache
directory (default: %LOCALAPPDATA%/slopshop/bench-models, or ~/.cache/slopshop/bench-models),
then its SHA-256 is checked; files already there and correct are skipped. No account or token
is needed: none of these repositories is gated.

    python fetch_models.py [--only <model>] [--dir <cache>]
"""

import argparse
import hashlib
import os
import sys
import urllib.request
from pathlib import Path


def default_dir() -> Path:
    base = os.environ.get("LOCALAPPDATA")
    root = Path(base) if base else Path.home() / ".cache"
    return root / "slopshop" / "bench-models"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--only", help="fetch only the files of this model (first column)")
    parser.add_argument("--dir", type=Path, default=default_dir())
    args = parser.parse_args()
    manifest = Path(__file__).with_name("models.tsv")
    rows = [
        line.split("\t")
        for line in manifest.read_text(encoding="utf-8").splitlines()
        if line and not line.startswith("#")
    ]
    failures = 0
    for model, repo, revision, path, size, expected in rows:
        if args.only and model != args.only:
            continue
        target = args.dir / repo / path
        if target.exists() and sha256(target) == expected:
            print(f"ok      {repo}/{path}")
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        url = f"https://huggingface.co/{repo}/resolve/{revision}/{path}"
        print(f"fetch   {repo}/{path} ({int(size) / 1e6:.0f} MB)", flush=True)
        partial = target.with_name(target.name + ".part")
        urllib.request.urlretrieve(url, partial)
        if sha256(partial) != expected:
            partial.unlink()
            print(f"BAD SHA {repo}/{path}", file=sys.stderr)
            failures += 1
            continue
        partial.replace(target)
    print(f"models in {args.dir}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
