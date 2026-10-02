"""Generates `src/manifest.rs`: what the AI installer downloads, pinned (ADR 0025).

For a file inside a zip archive (ONNX Runtime's release, NVIDIA's wheels), the manifest records
where its compressed bytes lie in the archive, so the installer fetches only them (an HTTP range)
and inflates them; for a plain file (a model on Hugging Face), the file itself. Every installed
file is pinned by its size and SHA-256. Sources are immutable: release assets, PyPI files, and
Hugging Face files at a commit.

    python crates/slopshop-ai/tools/manifest.py [--cache <folder of already-extracted files>]

Files already extracted under `--cache` (same name, same CRC-32 as in the archive) are hashed
locally instead of downloaded.
"""

import argparse
import hashlib
import json
import os
import struct
import urllib.request
import zlib

USER_AGENT = "slopshop-manifest"

ORT = "1.30.0"
ORT_URL = "https://github.com/microsoft/onnxruntime/releases/download/v{v}/{name}"
PYPI = "https://pypi.org/pypi/{package}/{version}/json"
HF = "https://huggingface.co/{repo}/resolve/{revision}/{path}"

# id: (name, url, commercial use allowed, must be accepted explicitly: not permissive open source)
LICENSES = {
    "onnxruntime": (
        "MIT",
        "https://github.com/microsoft/onnxruntime/blob/main/LICENSE",
        True,
        False,
    ),
    "cuda": (
        "NVIDIA CUDA Toolkit EULA",
        "https://docs.nvidia.com/cuda/eula/index.html",
        True,
        True,
    ),
    "cudnn": (
        "NVIDIA cuDNN Software License Agreement",
        "https://docs.nvidia.com/deeplearning/cudnn/backend/latest/reference/eula.html",
        True,
        True,
    ),
    "sam2": (
        "Apache-2.0",
        "https://github.com/facebookresearch/sam2/blob/main/LICENSE",
        True,
        False,
    ),
    # The weights' card (hustvl, the authors) says Apache-2.0; Xenova's repository is their
    # ONNX export.
    "vitmatte": (
        "Apache-2.0",
        "https://huggingface.co/hustvl/vitmatte-small-composition-1k",
        True,
        False,
    ),
}


def request(url, start=None, end=None):
    headers = {"User-Agent": USER_AGENT}
    if start is not None:
        headers["Range"] = f"bytes={start}-{end}"
    return urllib.request.urlopen(urllib.request.Request(url, headers=headers))


def length(url):
    r = urllib.request.urlopen(
        urllib.request.Request(url, method="HEAD", headers={"User-Agent": USER_AGENT})
    )
    return int(r.headers["Content-Length"])


def central_directory(url):
    """{name: (method, crc, compressed, size, local header offset)} of a remote zip."""
    total = length(url)
    tail = request(url, max(0, total - 65536), total - 1).read()
    eocd = tail.rfind(b"PK\x05\x06")
    count, cd_size, cd_offset = struct.unpack_from("<HII", tail, eocd + 10)
    if cd_offset == 0xFFFFFFFF:
        raise SystemExit(f"{url}: zip64 archives are not supported")
    cd = request(url, cd_offset, cd_offset + cd_size - 1).read()
    entries, at = {}, 0
    for _ in range(count):
        (method, _t, _d, crc, compressed, size, name_len, extra_len, comment_len) = (
            struct.unpack_from("<HHHIIIHHH", cd, at + 10)
        )
        offset = struct.unpack_from("<I", cd, at + 42)[0]
        name = cd[at + 46 : at + 46 + name_len].decode()
        entries[name] = (method, crc, compressed, size, offset)
        at += 46 + name_len + extra_len + comment_len
    return entries


def data_offset(url, header):
    local = request(url, header, header + 29).read()
    name_len, extra_len = struct.unpack_from("<HH", local, 26)
    return header + 30 + name_len + extra_len


def entry_hash(url, entry, cache):
    method, crc, compressed, size, header = entry
    offset = data_offset(url, header)
    if cache and os.path.exists(cache) and os.path.getsize(cache) == size:
        data = open(cache, "rb").read()
        if zlib.crc32(data) == crc:
            return offset, hashlib.sha256(data).hexdigest()
    raw = request(url, offset, offset + compressed - 1).read()
    data = zlib.decompress(raw, -15) if method == 8 else raw
    assert len(data) == size and zlib.crc32(data) == crc, url
    return offset, hashlib.sha256(data).hexdigest()


def archive_files(url, names, folder, cache_dir):
    entries = central_directory(url)
    # A name ending with "/": every library in that folder.
    names = [
        found
        for name in names
        for found in (
            sorted(e for e in entries if e.startswith(name) and e.endswith(".dll"))
            if name.endswith("/")
            else [name]
        )
    ]
    files = []
    for name in names:
        method, crc, compressed, size, header = entries[name]
        assert method in (0, 8), f"{name}: compression method {method}"
        base = name.rsplit("/", 1)[-1]
        cache = os.path.join(cache_dir, base) if cache_dir else None
        offset, sha = entry_hash(url, entries[name], cache)
        source = (
            f"Source::Deflated {{ offset: {offset}, compressed: {compressed} }}"
            if method == 8
            else f"Source::Stored {{ offset: {offset} }}"
        )
        files.append((f"{folder}/{base}", url, source, size, sha))
        print(f"  {base}: {size} bytes ({compressed} compressed)")
    return files


def wheel(package, version, names, folder, cache_dir):
    info = json.load(request(PYPI.format(package=package, version=version)))
    (url,) = [u["url"] for u in info["urls"] if u["filename"].endswith("win_amd64.whl")]
    return archive_files(url, names, folder, cache_dir)


def hugging_face(repo, revision, names, folder):
    tree = json.load(
        request(f"https://huggingface.co/api/models/{repo}/tree/{revision}/onnx?expand=true")
    )
    by_path = {f["path"]: f for f in tree}
    files = []
    for name in names:
        f = by_path[name]
        sha = (f.get("lfs") or {}).get("oid")
        assert sha, f"{name} is not stored with LFS: no SHA-256 to pin"
        url = HF.format(repo=repo, revision=revision, path=name)
        files.append((f"{folder}/{repo}/{name}", url, "Source::File", f["size"], sha))
    return files


def component(id, licenses, files):
    return id, licenses, files


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cache", help="folder of already-extracted runtime files")
    args = parser.parse_args()
    cache = args.cache

    print("ONNX Runtime, CUDA")
    cuda_name = f"onnxruntime-win-x64-gpu_cuda13-{ORT}"
    cuda = archive_files(
        ORT_URL.format(v=ORT, name=f"{cuda_name}.zip"),
        [
            f"{cuda_name}/lib/{dll}"
            for dll in [
                "onnxruntime.dll",
                "onnxruntime_providers_shared.dll",
                "onnxruntime_providers_cuda.dll",
            ]
        ],
        "runtime/cuda",
        None,
    )
    print("CUDA runtime, cuBLAS")
    cuda += wheel(
        "nvidia-cuda-runtime",
        "13.4.92",
        ["nvidia/cu13/bin/x86_64/cudart64_13.dll"],
        "runtime/cuda",
        cache,
    )
    cuda += wheel(
        "nvidia-cublas",
        "13.8.0.4",
        ["nvidia/cu13/bin/x86_64/cublas64_13.dll", "nvidia/cu13/bin/x86_64/cublasLt64_13.dll"],
        "runtime/cuda",
        cache,
    )
    print("cuDNN")
    # Every library of cuDNN: it loads its sublibraries by name, as its operations need them.
    cuda += wheel("nvidia-cudnn-cu13", "9.27.0.42", ["nvidia/cudnn/bin/"], "runtime/cuda", cache)
    print("ONNX Runtime, CPU")
    cpu_name = f"onnxruntime-win-x64-{ORT}"
    cpu = archive_files(
        ORT_URL.format(v=ORT, name=f"{cpu_name}.zip"),
        [f"{cpu_name}/lib/onnxruntime.dll"],
        "runtime/cpu",
        None,
    )
    print("SAM 2.1")
    sam_gpu = hugging_face(
        "onnx-community/sam2.1-hiera-base-plus-ONNX",
        "bab18593f44e652f04cf18b60b3690f60e8996b0",
        [
            f"onnx/{name}"
            for name in [
                "vision_encoder_fp16.onnx",
                "vision_encoder_fp16.onnx_data",
                "prompt_encoder_mask_decoder_fp16.onnx",
                "prompt_encoder_mask_decoder_fp16.onnx_data",
            ]
        ],
        "models",
    )
    sam_cpu = hugging_face(
        "onnx-community/sam2.1-hiera-tiny-ONNX",
        "814a066640debee5a91e70aa401fb8e17e030503",
        [
            f"onnx/{name}"
            for name in [
                "vision_encoder.onnx",
                "vision_encoder.onnx_data",
                "prompt_encoder_mask_decoder.onnx",
                "prompt_encoder_mask_decoder.onnx_data",
            ]
        ],
        "models",
    )
    print("ViTMatte-S")
    vitmatte = hugging_face(
        "Xenova/vitmatte-small-composition-1k",
        "6bc1297f6140f055a227b6d2cfe8c093281f35d2",
        ["onnx/model.onnx"],
        "models",
    )
    components = [
        component("runtime-cuda", ["onnxruntime", "cuda", "cudnn"], cuda),
        component("runtime-cpu", ["onnxruntime"], cpu),
        component("sam2.1-base-plus", ["sam2"], sam_gpu),
        component("sam2.1-tiny", ["sam2"], sam_cpu),
        component("vitmatte-small", ["vitmatte"], vitmatte),
    ]

    out = [
        "//! What the AI installer downloads, pinned (ADR 0025). Generated by `tools/manifest.py`:",
        "//! do not edit by hand.",
        "",
        "use crate::install::{Component, Download, License, Source};",
        "",
    ]
    for key, (name, url, commercial, accept) in LICENSES.items():
        out.append(
            f"const {key.upper()}: License = License {{ name: {json.dumps(name)}, "
            f"url: {json.dumps(url)}, commercial: {str(commercial).lower()}, "
            f"accept: {str(accept).lower()} }};"
        )
    out.append("")
    out.append("/// Every component, by id.")
    out.append("pub const COMPONENTS: &[Component] = &[")
    for id, licenses, files in components:
        out.append("    Component {")
        out.append(f"        id: {json.dumps(id)},")
        out.append(f"        licenses: &[{', '.join(l.upper() for l in licenses)}],")
        out.append("        files: &[")
        for path, url, source, size, sha in files:
            out.append("            Download {")
            out.append(f"                path: {json.dumps(path)},")
            out.append(f"                url: {json.dumps(url)},")
            out.append(f"                source: {source},")
            out.append(f"                size: {size},")
            out.append(f"                sha256: {json.dumps(sha)},")
            out.append("            },")
        out.append("        ],")
        out.append("    },")
    out.append("];")
    here = os.path.dirname(os.path.abspath(__file__))
    target = os.path.join(here, "..", "src", "manifest.rs")
    with open(target, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print("written", os.path.normpath(target))


if __name__ == "__main__":
    main()
