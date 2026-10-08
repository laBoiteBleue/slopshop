"""Generates `src/manifest.rs`: what the AI installer downloads, pinned (ADR 0025).

For a file inside a zip archive (ONNX Runtime's DirectML package), the manifest records
where its compressed bytes lie in the archive, so the installer fetches only them (an HTTP range)
and inflates them; for a file inside a gzip-compressed tar archive (ONNX Runtime's macOS and
Linux releases), the archive's size and the entry's name (it is fetched whole); for a plain
file (a model on Hugging Face), the file itself. Every installed
file is pinned by its size and SHA-256. Sources are immutable: release assets, PyPI files, and
Hugging Face files at a commit.

    python crates/slopshop-ai/tools/manifest.py [--cache <folder of already-extracted files>]

Files already extracted under `--cache` (same name, same CRC-32 as in the archive) are hashed
locally instead of downloaded.
"""

import argparse
import hashlib
import io
import json
import os
import struct
import tarfile
import urllib.request
import zlib

USER_AGENT = "slopshop-manifest"

# ONNX Runtime with DirectML: Microsoft's last DirectML release (DirectML is in maintenance),
# from its PyPI package (a zip holding the libraries, whatever Python it targets).
ORT_DIRECTML = ("onnxruntime-directml", "1.24.4", "cp312-cp312-win_amd64")
PYPI = "https://pypi.org/pypi/{package}/{version}/json"
# ONNX Runtime's macOS (Core ML built in) and Linux (CPU) releases.
ORT = "1.30.0"
ORT_URL = "https://github.com/microsoft/onnxruntime/releases/download/v{v}/{name}"
HF = "https://huggingface.co/{repo}/resolve/{revision}/{path}"

# id: (name, url, commercial use allowed, must be accepted explicitly: not permissive open source)
LICENSES = {
    "onnxruntime": (
        "MIT",
        "https://github.com/microsoft/onnxruntime/blob/main/LICENSE",
        True,
        False,
    ),
    "directml": (
        "Microsoft DirectML License",
        "https://www.nuget.org/packages/Microsoft.AI.DirectML/1.15.4/License",
        True,
        True,
    ),
    "sam2": (
        "Apache-2.0",
        "https://github.com/facebookresearch/sam2/blob/main/LICENSE",
        True,
        False,
    ),
    "birefnet": (
        "MIT",
        "https://github.com/ZhengPeng7/BiRefNet/blob/main/LICENSE",
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
    "vitmatte_base": (
        "Apache-2.0",
        "https://huggingface.co/hustvl/vitmatte-base-composition-1k",
        True,
        False,
    ),
    "flux2_klein": (
        "Apache-2.0",
        "https://huggingface.co/black-forest-labs/FLUX.2-klein-4B",
        True,
        False,
    ),
    # The Erase tool's LoRA, distilled from Klein base 4B and fal's object-remove LoRA (both
    # Apache-2.0), on images of the CORNE set (Apache-2.0).
    "erase_v1": (
        "Apache-2.0",
        "https://huggingface.co/slopshop/erase-v1",
        True,
        False,
    ),
}

# The Erase tool (ADR 0045): FLUX.2 [klein] 4B turbo as BFL publishes it, and erase_v1.
FLUX2_KLEIN = ("black-forest-labs/FLUX.2-klein-4B", "e7b7dc27f91deacad38e78976d1f2b499d76a294")
ERASE_V1 = ("slopshop/erase-v1", "0bb45298a0f4dde0affed10c73f8ad05ad26de48")


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


def wheel(package, version, tag, names, folder, cache_dir):
    info = json.load(request(PYPI.format(package=package, version=version)))
    (url,) = [u["url"] for u in info["urls"] if u["filename"].endswith(f"{tag}.whl")]
    return archive_files(url, names, folder, cache_dir)


def tgz_file(url, suffix, path):
    """The entry of a .tar.gz release whose name ends with `suffix`, installed at `path`."""
    data = request(url).read()
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        (member,) = [m for m in archive.getmembers() if m.isfile() and m.name.endswith(suffix)]
        content = archive.extractfile(member).read()
    print(f"  {member.name}: {len(content)} bytes ({len(data)} archive)")
    source = f"Source::TarGz {{ archive: {len(data)}, entry: {json.dumps(member.name)} }}"
    return [(path, url, source, len(content), hashlib.sha256(content).hexdigest())]


def hugging_face(repo, revision, names, folder):
    by_path = {}
    for directory in sorted({os.path.dirname(name) for name in names}):
        tree = f"https://huggingface.co/api/models/{repo}/tree/{revision}"
        if directory:
            tree += f"/{directory}"
        by_path.update({f["path"]: f for f in json.load(request(tree + "?expand=true"))})
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

    print("ONNX Runtime, DirectML")
    package, version, tag = ORT_DIRECTML
    directml = wheel(
        package,
        version,
        tag,
        [
            f"onnxruntime/capi/{dll}"
            for dll in ["onnxruntime.dll", "onnxruntime_providers_shared.dll", "DirectML.dll"]
        ],
        "runtime/directml",
        cache,
    )
    print("ONNX Runtime, macOS (Core ML), Linux (CPU)")
    coreml = tgz_file(
        ORT_URL.format(v=ORT, name=f"onnxruntime-osx-arm64-{ORT}.tgz"),
        f"lib/libonnxruntime.{ORT}.dylib",
        "runtime/coreml/libonnxruntime.dylib",
    )
    linux_x64 = tgz_file(
        ORT_URL.format(v=ORT, name=f"onnxruntime-linux-x64-{ORT}.tgz"),
        f"lib/libonnxruntime.so.{ORT}",
        "runtime/cpu-x64/libonnxruntime.so",
    )
    linux_arm64 = tgz_file(
        ORT_URL.format(v=ORT, name=f"onnxruntime-linux-aarch64-{ORT}.tgz"),
        f"lib/libonnxruntime.so.{ORT}",
        "runtime/cpu-arm64/libonnxruntime.so",
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
    sam_fp32 = hugging_face(
        "onnx-community/sam2.1-hiera-base-plus-ONNX",
        "bab18593f44e652f04cf18b60b3690f60e8996b0",
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
    sam_tiny = hugging_face(
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
    print("BiRefNet")
    birefnet = hugging_face(
        "onnx-community/BiRefNet-ONNX",
        "534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67",
        ["onnx/model_fp16.onnx"],
        "models",
    )
    birefnet_fp32 = hugging_face(
        "onnx-community/BiRefNet-ONNX",
        "534d3c82d3bb8b2f0867db6dfbc3a525b8e42f67",
        ["onnx/model.onnx"],
        "models",
    )
    birefnet_lite = hugging_face(
        "onnx-community/BiRefNet_lite-ONNX",
        "de15b22ba131738a16dff04aab8bdf8dc32e3ac1",
        ["onnx/model.onnx"],
        "models",
    )
    print("ViTMatte-S")
    vitmatte = hugging_face(
        "Xenova/vitmatte-small-composition-1k",
        "6bc1297f6140f055a227b6d2cfe8c093281f35d2",
        ["onnx/model.onnx"],
        "models",
    )
    print("ViTMatte-B")
    vitmatte_base = hugging_face(
        "Xenova/vitmatte-base-composition-1k",
        "1290b014b994e95ca1b9dd9c5f72c3b6d5b7236a",
        ["onnx/model.onnx"],
        "models",
    )
    print("FLUX.2 [klein] 4B, erase_v1")
    flux2_klein = hugging_face(
        *FLUX2_KLEIN,
        [
            "transformer/diffusion_pytorch_model.safetensors",
            "vae/diffusion_pytorch_model.safetensors",
        ],
        "models",
    )
    erase_v1 = hugging_face(
        *ERASE_V1,
        ["erase_v1_diffusers.safetensors", "prompt_embeds.safetensors"],
        "models",
    )
    components = [
        # Windows: DirectML, the models in half precision.
        component("runtime-directml", ["onnxruntime", "directml"], directml),
        component("sam2.1-base-plus", ["sam2"], sam_gpu),
        component("birefnet", ["birefnet"], birefnet),
        # macOS (Apple silicon): Core ML, the models in single precision.
        component("runtime-coreml-macos-arm64", ["onnxruntime"], coreml),
        component("sam2.1-base-plus-fp32", ["sam2"], sam_fp32),
        component("birefnet-fp32", ["birefnet"], birefnet_fp32),
        # Linux: the CPU, the small models.
        component("runtime-cpu-linux-x64", ["onnxruntime"], linux_x64),
        component("runtime-cpu-linux-arm64", ["onnxruntime"], linux_arm64),
        component("sam2.1-tiny", ["sam2"], sam_tiny),
        component("birefnet-lite", ["birefnet"], birefnet_lite),
        # Refine Edge: the base model on a GPU, the small one on the CPU (2 s a window for
        # the base model there).
        component("vitmatte-small", ["vitmatte"], vitmatte),
        component("vitmatte-base", ["vitmatte_base"], vitmatte_base),
        # The Erase tool, on DirectML (ADR 0045).
        component("flux2-klein-4b", ["flux2_klein"], flux2_klein),
        component("erase-v1", ["erase_v1"], erase_v1),
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
