# Research: AI selection (click, box, subject, text, edge refinement)

Status: **research brief**, being confirmed by experiment (`experiments/ai-bench/`). Nothing
described here is in the product yet.

Maintainer's decisions so far (2026-10-01), to be recorded in an ADR with the measurements:
- the inference runs in a **separate `slopshop-ai` helper process**, through ONNX Runtime (`ort`),
  like `slopshop-raw` (ADR 0023);
- **weights usable commercially are accepted**, SAM 3 included (not only OSI-permissive ones);
- on Windows, **TensorRT for RTX downloaded on demand** on NVIDIA RTX cards, **DirectML**
  otherwise;
- models are **downloaded on first use**, with consent, pinned and checked (section 5).

Questions 5 to 7 of section 8 are still open.
State of the facts: **2026-10-01**. Sources are linked inline. "(unverified)" marks claims
that could not be confirmed from a primary source. "(est.)" marks numbers derived by us, for
example a size computed from a parameter count.

Scope: the first selection delivery should include AI selection: click to select (positive and
negative points), box prompt, "Select Subject", semantic or text selection ("sky", "person",
"the red car"), and ideally edge refinement (hair). The selection is a 16-bit gray mask at
document level. AI operations are nodes (model + version, seed, prompt, mask, ROI, cached
result, invalidation rather than silent recompute; see
[hd-generative-ai.md](hd-generative-ai.md) and [architecture.md](../architecture.md)).

---

## 1. Summary

- **Runtime: ONNX Runtime through the `ort` crate** (2.0.0-rc.13, July 2026, wraps ONNX Runtime
  1.30.0, Sept 2026). No other Rust option today covers every model below at GPU speed on
  Windows, macOS and Linux. Execution providers (EPs) to use on Windows: **TensorRT for RTX**
  (NVIDIA RTX, about 200 MB, engines compiled on the user's machine), then **DirectML** (any
  DirectX 12 GPU; "legacy" but still shipped), then CPU. Watch the WebGPU EP (Dawn: D3D12,
  Vulkan, Metal), still experimental. Pure-Rust runtimes (burn, candle) and wgpu-based ones
  are not ready for SAM 2 / BiRefNet-class models at product quality: wonnx is archived, and
  burn's ONNX import is maturing but unproven on these models.
- **Run inference in a helper process** (`slopshop-ai`, same pattern as `slopshop-raw`,
  ADR 0023), kept alive while the tool is in use. This keeps native and proprietary GPU
  libraries (hundreds of MB) out of the editor binary and contains driver or EP crashes and
  out-of-memory errors. It also frees all VRAM when the helper exits. Only small inputs cross
  the pipe (≤ 2048² RGB) and only low-resolution logits come back.
- **Permissive models exist for every feature except top-quality text selection:**
  - click and box: **SAM 2.1** (Apache-2.0 code and weights);
  - Select Subject: **BiRefNet** (MIT; HR and dynamic variants up to 2048–2304 px);
  - text: **Grounding DINO** (Apache-2.0) or **Florence-2** (MIT) for boxes, then SAM 2.1 for
    masks (the "Grounded-SAM-2" pattern, Apache-2.0);
  - edges: a classical edge-aware refinement at full resolution first, then BiRefNet-matting
    (MIT) or CascadePSP (MIT) as learned refiners.
- **SAM 3 is the best text-to-mask model, but its license is not open source.** The SAM
  License allows commercial use and redistribution but forbids military, nuclear and espionage
  uses and requires trade-control compliance. Downloads on Hugging Face are gated. It is also
  large (848 M parameters, 3.45 GB checkpoint). This is a **maintainer decision**: SAM 3 could
  be an optional download that the user accepts explicitly.
- **Excluded by license:** RMBG-2.0 (CC BY-NC 4.0), MatAnyone (S-Lab, non-commercial),
  EfficientSAM3 (derived from SAM 3, and its MobileCLIP text encoder is under Apple's
  research-only license), EdgeSAM (S-Lab, unverified), YOLOE / Ultralytics (AGPL).
- **Huge images:** run the encoder on a ≈1024 px pyramid level, or on the zoomed ROI. Then
  refine only the **boundary band** at full resolution, tile by tile (edge-aware filter, then
  optional learned matting). Interior and exterior tiles are constant, so a 500 MP mask costs
  about as much as its outline.
- **Models are downloaded on first use**, never bundled by default. The app keeps a manifest
  pinned by upstream commit, checked by SHA-256, and stored in a per-user cache. A document
  stores the resulting mask plus its provenance, because GPU inference is not bit-reproducible
  across machines.

---

## 2. Inference runtime options in Rust

### 2.1 ONNX Runtime via `ort`

| Item | Fact (date) |
|---|---|
| Crate version | `ort` **2.0.0-rc.13**, released 2026-07-28; previous rc.12 2026-03-05. Described upstream as "production-ready" despite the rc tag ([ort docs](https://ort.pyke.io/), [releases](https://github.com/pykeio/ort/releases)). |
| ONNX Runtime | **1.30.0**, released 2026-09-10 ([GitHub](https://github.com/microsoft/onnxruntime/releases), [PyPI](https://pypi.org/project/onnxruntime/)). ort's prebuilt binaries are built from `ms@1.30.0` ([dist.tsv](https://github.com/pykeio/ort/blob/main/ort-sys/build/download/dist.tsv)). |
| Licenses | ort: MIT OR Apache-2.0 (Cargo.toml). ONNX Runtime: MIT. Both pass `cargo deny`. **The EP runtimes are not covered by `cargo deny`:** TensorRT-RTX and CUDA are NVIDIA proprietary EULAs; OpenVINO uses an Intel distribution license. |
| Prebuilt binaries (ort) | Statically linked, need x86-64-v3 ([prebuilt binaries](https://ort.pyke.io/misc/prebuilt-binaries)). Windows x64 builds: `directml`, `webgpu`, `nvrtx,directml`, `cuda13,tensorrt,nvrtx,directml`. macOS arm64: `coreml`, `coreml,webgpu`. Linux x64: `none`, `webgpu`, `nvrtx`, `cuda13,tensorrt,nvrtx`. Every Windows build includes DirectML and every macOS build includes CoreML. |
| Linking strategies | Default `download-binaries` (static). The **`load-dynamic`** feature loads `onnxruntime.dll` at run time from a path chosen by the app; upstream recommends it, and it allows failing gracefully when AI is not installed. `copy-dylibs` copies the EP DLLs next to the binary. CUDA and TensorRT EPs are always separate dynamic libraries ([linking](https://ort.pyke.io/setup/linking), [EPs](https://ort.pyke.io/perf/execution-providers)). |
| Plugin EPs | ort exposes `Environment::register_ep_library(name, path)` (ORT API ≥ 22). It loads an EP DLL at run time, such as one Windows ML downloaded or one we downloaded ourselves (ort `src/environment.rs`, main branch). |
| CUDA EP | Prebuilt for CUDA 13; needs cuDNN 9.x on the PATH ([EPs](https://ort.pyke.io/perf/execution-providers)). ORT 1.28 made cuDNN and cuFFT optional for the CUDA EP (release notes). The `onnxruntime-gpu` wheel alone weighs 160 MB on Windows and 247 MB on Linux, without the CUDA runtime libraries ([PyPI](https://pypi.org/project/onnxruntime-gpu/)). With the CUDA runtime, cuBLAS and cuDNN, a CUDA install reaches **1–2 GB** (unverified). |
| TensorRT for RTX EP (`nvrtx`) | NVIDIA's client-side TensorRT. RTX GPUs from Turing (CC 7.5) to Blackwell (CC 12.x), so the RTX 5070 Ti is covered. Library "just under 200 MB" (JIT part < 100 MB). AOT compile < 15 s, then a device-specific JIT pass of "a few seconds", with a run-time cache ([NVIDIA blog](https://developer.nvidia.com/blog/nvidia-tensorrt-for-rtx-introduces-an-optimized-inference-ai-library-on-windows/), [ORT docs](https://onnxruntime.ai/docs/execution-providers/TensorRTRTX-ExecutionProvider.html)). Whether ort's `nvrtx` build ships the TensorRT-RTX DLLs or expects them installed is **unverified**; first experiment. |
| DirectML EP | "Sustained engineering": still supported, but new work has moved to Windows ML, which lists DirectML as **legacy** ([MS Learn, updated 2026-09-28](https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/supported-execution-providers), [issue #23783](https://github.com/microsoft/onnxruntime/issues/23783)). Works on any DX12 GPU (NVIDIA, AMD, Intel). The last `onnxruntime-directml` wheel on PyPI is **1.24.4 (2026-03-17)**, 25 MB, which hints that DirectML is no longer released on its own ([PyPI](https://pypi.org/project/onnxruntime-directml/)). ort still builds it into 1.30 binaries. |
| Windows ML | ORT shipped with Windows (Windows App SDK). EPs are downloaded on demand through `ExecutionProviderCatalog`: NvTensorRtRtx (RTX 30xx+), MIGraphX (AMD RDNA 3+), OpenVINO (Intel), QNN, VitisAI, and WebGPU (experimental, 2.x only). Requires Windows 11 24H2 (build 26100)+ ([MS Learn](https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/supported-execution-providers)). Using it from Rust would mean the Windows App SDK bootstrap plus `register_ep_library`; **not tried by anyone we found** (unverified). |
| WebGPU EP | Built on Dawn: D3D12 or Vulkan on Windows, Vulkan on Linux, Metal on Apple. "A single build can target any GPU" without a vendor SDK. Packaged as a plugin EP (`onnxruntime_providers_webgpu`, plugin releases up to v0.4.0). Graph capture needs static shapes ([ORT docs](https://onnxruntime.ai/docs/execution-providers/WebGPU-ExecutionProvider.html)). **ort marks it experimental**: it "may produce incorrect results/crashes" ([ort EPs](https://ort.pyke.io/perf/execution-providers)). It is the only ORT path to cross-vendor GPUs on Linux and to AMD/Intel GPUs without a vendor EP. |
| CoreML EP (macOS) | Included in every ort macOS build. Operator coverage for SAM 2 and BiRefNet graphs is unverified; unsupported nodes fall back to CPU. CI only (the maintainer cannot test macOS). |
| `unsafe` | All FFI is inside `ort`/`ort-sys`; the public API is safe. The workspace `unsafe_code = deny` lint applies only to our crates. |
| Rust references | **usls** (MIT, [GitHub](https://github.com/jamjamjon/usls)) is a Rust vision library on ort. It already runs SAM, SAM-HQ, MobileSAM, SAM 2, SAM 3 tracker, BiRefNet, BEN2, RMBG and MODNet on CUDA, TensorRT, TensorRT-RTX, CoreML, OpenVINO and DirectML. Useful as a reference for preprocessing and postprocessing; not proposed as a dependency (too broad, alpha). |

Wheel sizes give an idea of the runtime size: `onnxruntime` 1.30 CPU is 14.7 MB on Windows,
21.5 MB on macOS arm64 and 23.6 MB on Linux; the DirectML wheel is 25 MB.

### 2.2 Other runtimes

| Option | GPU coverage | Model coverage for this feature | Packaging | `unsafe`/FFI | Maintenance | Verdict |
|---|---|---|---|---|---|---|
| **burn** (Apache/MIT) 0.21 (2026-05-07), 0.22.0-pre.4 (2026-09-22); burn-onnx 0.21 (2026-05-14) ([blog](https://burn.dev/blog/)) | CubeCL backends: CUDA, ROCm, Metal, Vulkan, WebGPU/wgpu (DX12). True cross-vendor GPU in pure Rust. | burn-onnx turns ONNX into Rust code. "27 real-world model checks", but SAM 2 / BiRefNet are **not known to import**. BiRefNet uses deformable convolution, and attention-heavy ViTs may be slow on wgpu (unverified). | Small, no native libraries | None in our code | Active, fast-moving API | Candidate for **a later spike** (pure Rust, could share the wgpu device). Not for the first delivery. |
| **candle** (Apache/MIT) 0.11.0 (2026-06) | CUDA, Metal; no official wgpu/Vulkan (a [fork](https://github.com/FerrisMind/candle) adds them) | Hand-written models: SAM v1 and MobileSAM exist in candle-transformers; no SAM 2/3, BiRefNet, Grounding DINO (unverified for the latest version). `candle-onnx` has limited operator coverage. | Small | None | Active | No: AMD/Intel GPUs on Windows would fall back to CPU, and every model must be ported by hand. |
| **wonnx** (wgpu ONNX) | wgpu | Old operator set | — | — | **Archived 2025-05-07** ([GitHub](https://github.com/webonnx/wonnx)) | Excluded. |
| **tract** (Sonos), 0.23.8 (2026-09-21) | CPU only | Good ONNX coverage | Pure Rust | — | Active | CPU fallback at most; ORT's CPU EP is a simpler fallback inside the same runtime. |
| **vision.cpp / ggml** (MIT, [GitHub](https://github.com/Acly/vision.cpp)), used by Krita Vision Tools | CPU, **Vulkan** (NVIDIA, AMD, Intel) | MobileSAM, BiRefNet (+ Depth-Anything, MI-GAN, ESRGAN). No SAM 2/3 and no text models. Models must be converted to GGUF. | "<5 MB CPU, +30 MB GPU"; model load < 100 ms | C++ library: we would write the FFI bindings (`unsafe`) | Single maintainer, 58 stars | Interesting proof that a light Vulkan path works (numbers in §3.6). Too narrow for SAM 2 and text. |
| **Python sidecar** (PyTorch) | CUDA, ROCm (Linux), MPS | Everything, from day one (official SAM 3 code) | PyTorch CUDA alone is several GB (unverified); slow start | Process boundary | — | **Experiments only** (`experiments/`, already allowed by hd-generative-ai.md): export models, reference outputs, quality baselines. Not in the product. |

### 2.3 Trade-offs that drive the choice

- **GPU performance:** TensorRT-class EPs are clearly faster than generic paths for ViT
  encoders. For example, BiRefNet on an RTX 4080S runs in 0.11 s with TensorRT versus 0.15 s
  with PyTorch ([BiRefNet README](https://github.com/ZhengPeng7/BiRefNet)). FP16 matters a
  lot: the SAM 3 encoder at 1008 px takes 390 ms in f32 versus 110 ms in bf16 on an RTX 3090
  ([sam3 #424](https://github.com/facebookresearch/sam3/issues/424)).
- **Hidden CPU fallback:** an exported graph can be dramatically slower when some nodes fall
  back to CPU. The same BiRefNet comparison shows 4.43 s with ONNX, likely an old opset where
  DeformConv runs on CPU. BiRefNet's README says the native `DeformConv` in opset 22 is much
  faster. **Every model must be checked for per-node EP placement.**
- **Cross-vendor coverage on Windows without CUDA:** DirectML today, and the WebGPU EP or
  Windows ML vendor EPs (MIGraphX, OpenVINO) tomorrow. Only burn and ggml give it in a
  "single portable" way, and their model coverage is narrow.
- **Packaging:** ORT core is about 15–25 MB, DirectML adds little, TensorRT-RTX adds about
  200 MB, CUDA adds 1–2 GB (unverified). Shipping NVIDIA binaries inside an open-source
  installer raises a license policy question: proprietary EULA, redistributable but not
  OSI. Downloading on demand avoids putting them in the installer.
- **GPU sharing with wgpu:** zero-copy interop between ORT (CUDA/D3D12 EP) and our wgpu device
  would need external-memory APIs and `unsafe`. It is **not needed**: model inputs are at most
  2048² × 3 bytes (12.6 MB) and outputs are 256²–1024² logits, so a CPU round-trip costs a few
  milliseconds. VRAM is shared, though: BiRefNet needs 3.5 GB in FP16 under PyTorch, and the
  display cache has its own budget. The helper must unload sessions when idle.

---

## 3. Models

Latency figures are as published, on the hardware named. No figure exists for the RTX 5070 Ti
(Blackwell, 16 GB); our benchmark must measure them (§7).

### 3.1 Promptable segmentation (click, box)

| Model | License (code / weights) | Size | ONNX | Input | Published speed | Notes |
|---|---|---|---|---|---|---|
| **SAM 2.1** Hiera T/S/B+/L ([GitHub](https://github.com/facebookresearch/sam2)) | Apache-2.0 / **Apache-2.0** | 38.9 M / 46 M / 80.8 M / 224.4 M params. Checkpoints 156 / 184 / 324 / 898 MB ([HF](https://huggingface.co/facebook/sam2.1-hiera-large)) | Yes: [onnx-community](https://huggingface.co/onnx-community/sam2.1-hiera-large-ONNX) (encoder fp16 67 MB (T) … 444 MB (L)), [samexporter](https://github.com/vietanhdev/samexporter) (MIT) | 1024×1024; decoder outputs **256×256 low-res logits** upscaled to the input | 91 / 85 / 64 / 40 FPS on A100 (video, PyTorch) | **Primary candidate.** Encoder once per image or ROI; decoder per click (milliseconds). Released 2024-09-30; no newer image-only Apache successor from Meta. |
| **HQ-SAM / HQ-SAM 2** ([GitHub](https://github.com/SysCV/sam-hq)) | Apache-2.0 / Apache-2.0 | SAM + a small HQ token head | Partial (usls runs SAM-HQ) | 1024 | ≈ SAM | Better thin structures and edges. HQ-SAM 2 (beta, 2024-11-17) is built on SAM 2. **Benchmark against SAM 2.1.** |
| **MobileSAM** / **SlimSAM** / **EfficientViT-SAM** / **EfficientSAM** | Apache-2.0 (all) | MobileSAM ≈10 M params; SlimSAM ONNX encoder 23 MB, fp16 12 MB ([HF](https://huggingface.co/Xenova/slimsam-77-uniform)) | Yes | 1024 | MobileSAM encoder 19 ms fp16 on RTX 4070 (vision.cpp, Vulkan) | **Low-end / CPU fallback.** Lower quality than SAM 2.1 (unverified on our set). |
| **EdgeTAM** ([GitHub](https://github.com/facebookresearch/EdgeTAM)) | Apache-2.0 / Apache-2.0 | 56 MB checkpoint | Community (Qualcomm AI Hub) | 1024 | 16 FPS on iPhone 15 Pro Max | Video-oriented SAM 2 distillation; brings little for still images over SAM 2.1-T. |
| **EdgeSAM** | S-Lab License (non-commercial), unverified (GitHub shows NOASSERTION) | — | — | — | — | **Excluded** unless the license is confirmed permissive. |
| **SAM 3 / 3.1** (points and boxes via its tracker) | **SAM License** (see §3.3) | 848 M params total; 3.45 GB checkpoint; tracker ONNX encoder 935 MB fp16 ([HF](https://huggingface.co/onnx-community/sam3-tracker-ONNX)) | Yes (community) | 1008 | see §3.3 | Overkill for clicks; consider only together with text. |

### 3.2 Salient object / Select Subject

| Model | License | Size | ONNX | Input | Published speed | Notes |
|---|---|---|---|---|---|---|
| **BiRefNet** (general) ([GitHub](https://github.com/ZhengPeng7/BiRefNet)) | **MIT / MIT** | Swin-L; 444.5 MB safetensors; ONNX 973 MB fp32 / 490 MB fp16 ([HF](https://huggingface.co/onnx-community/BiRefNet-ONNX)) | Yes (official release + onnx-community); use **opset ≥ 22** for native DeformConv | 1024 | RTX 4090: 95.8 ms fp32 / **57.7 ms fp16**, 3.5 GB VRAM fp16 (PyTorch). RTX 4070 Vulkan fp16: 208 ms (vision.cpp). ONNX on A100: ~165 ms | **Primary candidate.** |
| **BiRefNet_HR** / **BiRefNet_dynamic** | MIT | Same as BiRefNet (444.5 MB) | Convertible (official notebook) | HR: 2048²; dynamic: trained 256²–2304², "any resolution" (2025-02-01 / 2025-03-31) | ~4× the 1024 cost at 2048 (est.) | Best fit for **large images**: more detail per pass. |
| **BiRefNet_lite** (Swin-T) / lite-2K | MIT | 177.6 MB; ONNX fp16 114.5 MB | Yes | 1024 / 2560×1440 | RTX 4070 Vulkan fp16 85 ms; ONNX A100 ~94 ms | Low-end tier. |
| **BEN2** ([HF](https://huggingface.co/PramaLLC/BEN2)) | MIT / MIT | ONNX 223 MB | Yes | 1024 (unverified) | — | Benchmark against BiRefNet. |
| **InSPyReNet** | MIT | — | Community | Multi-scale, designed for HR | — | Older (2022); optional comparison. |
| **U²-Net** | Apache-2.0 | 176 MB (u2net) / 4.7 MB (u2netp) (unverified) | Yes (rembg) | 320 | Fast | Outdated quality; not proposed. |
| **RMBG-2.0** (Bria) | **CC BY-NC 4.0**, commercial use needs an agreement; gated download ([HF](https://huggingface.co/briaai/RMBG-2.0)) | 1 GB ONNX | Yes | 1024 | — | **Excluded** (same architecture as BiRefNet, proprietary training). |

### 3.3 Open-vocabulary text → mask

| Model | License | Size | ONNX | Input | Published speed | Notes |
|---|---|---|---|---|---|---|
| **SAM 3 / SAM 3.1** ([GitHub](https://github.com/facebookresearch/sam3), [paper](https://arxiv.org/abs/2511.16719)) | **SAM License**: non-exclusive, royalty-free, **commercial use and redistribution allowed** with a copy of the license. **Forbids** military/warfare, nuclear, espionage and weapons uses and ITAR-controlled activities; requires trade-control compliance. Not OSI. **HF download gated (manual approval)** ([HF](https://huggingface.co/facebook/sam3)) | 848 M params; 3.45 GB fp32 (≈1.7 GB fp16, est.) | Community: 3 graphs (image encoder, CLIP-style text encoder, decoder) ([HF](https://huggingface.co/vietanhdev/segment-anything-3-onnx-models)) | 1008; text ≤ 32 tokens; noun phrases ("red car", "person with hat") | Meta: ~30 ms per image with 100+ objects on **H200**. RTX 3090 bf16: encoder 110 ms, text detection 22 ms. Via ORT TensorRT EP fp32: ~1.1 s reported ([#424](https://github.com/facebookresearch/sam3/issues/424)) | **Best quality**, including "stuff" (sky, water). Returns all instances. SAM 3.1 released 2026-03-27. Complex referring expressions ("the person on the left") need the SAM 3 Agent (an LLM on top). |
| **Grounding DINO** T / B ([GitHub](https://github.com/IDEA-Research/GroundingDINO)), MM-Grounding-DINO | Apache-2.0 / Apache-2.0 | Tiny 689 MB; base 933 MB; tiny ONNX fp16 360 MB ([HF](https://huggingface.co/onnx-community/grounding-dino-tiny-ONNX)) | Yes | 800×1333 | Tiny: 112 ms fp16 on A100 ([Roboflow](https://playground.roboflow.com/models/idea-research/grounding-dino)) | Text → **boxes**, then SAM 2.1 box prompts → masks ([Grounded-SAM-2](https://github.com/IDEA-Research/Grounded-SAM-2), Apache-2.0). Needs a BERT tokenizer (HF `tokenizers`, Apache-2.0). Good for "things", unverified for "stuff". |
| **Florence-2** B / L | MIT / MIT | 0.23 B / 0.77 B params; ONNX base ≈ 370 MB encoder + 390 MB decoder fp32 ([HF](https://huggingface.co/onnx-community/Florence-2-base)) | Yes (encoder + autoregressive decoder) | 768 | Not published for our case; autoregressive, so likely hundreds of ms (unverified) | Supports **referring expressions** → boxes, or polygons (35.8 mIoU on RefCOCO RES, coarse). Use its boxes with SAM 2.1. |
| **OWLv2** | Apache-2.0 / Apache-2.0 | 620 MB (base) | Yes | 960 | — | Text → boxes; alternative to Grounding DINO. |
| **CLIPSeg** | Code MIT; **weight license unclear** (the repo says MIT does not cover the weights; HF tags Apache-2.0) | 603 MB | Yes | 352 | Fast | Low-resolution heatmaps. **Excluded** until the license is clear. |
| **EfficientSAM3** ([HF](https://huggingface.co/Simon7108528/EfficientSAM3)) | Card says Apache-2.0, but the model is distilled from SAM 3 and fine-tuned on SAM 3 data, and its **MobileCLIP text encoders are under Apple's research-only license** ([LICENSE_MODELS](https://github.com/apple/ml-mobileclip)) | 0.7–22 M params (image encoders) | "Coming soon" | — | — | **Excluded** (license chain; no ONNX yet). |
| YOLOE / Ultralytics | AGPL-3.0 | — | — | — | — | **Excluded**. |

### 3.4 Matting and edge refinement

| Model or method | License | Size | Input | Notes |
|---|---|---|---|---|
| **Classical edge-aware refinement**: guided filter (He et al.), joint bilateral upsampling, fast bilateral solver | Algorithms, our code | — | Any, tiled | Upsamples the low-res mask along image edges at **full resolution** in wgpu compute shaders. Deterministic, no model, no license question. Weak on hair compared with learned matting. |
| **Foreground color estimation**: fast multi-level foreground estimation (Germer et al. 2020; [pymatting](https://github.com/pymatting/pymatting), MIT) | MIT | — | Any | For "decontaminate colors", not for the selection itself. BiRefNet made a GPU version (~80 ms on an RTX 5090). |
| **BiRefNet-matting / BiRefNet_HR-matting** | MIT / MIT | 885 MB (fp32) | 1024 / 2048 | Trimap-free alpha. Can run on boundary tiles. |
| **ViTMatte** S / B ([HF](https://huggingface.co/hustvl/vitmatte-small-composition-1k)) | Code MIT, weights tagged Apache-2.0; authors say MIT applies to the weights ([issue #9](https://github.com/hustvl/ViTMatte/issues/9)) | 103 MB (S) / 387 MB (B) | Trimap, any size (ViT, memory grows) | Trimap = eroded/dilated band of our mask. **Provenance caveat:** trained on Adobe Composition-1k (non-commercial dataset) with backbones pretrained on ImageNet; a 2025 comment in the issue argues the weights are therefore not commercially clean. |
| **CascadePSP** ([GitHub](https://github.com/hkchengrex/CascadePSP)) | MIT | Small (unverified) | **Any resolution** (global step, then local patches) | Designed exactly to refine coarse masks to very high resolution. Older (2020); compare with the classical method. |
| **SegRefiner** | Apache-2.0 | — | — | Diffusion-based, slow; not proposed. |
| **MatAnyone / MatAnyone 2** | **S-Lab License 1.0 (non-commercial)** | — | — | **Excluded**. |
| **MODNet** | Code Apache-2.0; weight license unverified | — | 512 | Portrait only; not proposed. |

**Training-data provenance (applies to almost every model above).** Declared weight licenses
are permissive, but many models were trained on research-only datasets (SA-1B for SAM, DIS5K,
P3M-10k and AM-2k for BiRefNet, Composition-1k for ViTMatte). The industry practice (Krita
plugins, GIMP plugins, ComfyUI) is to rely on the declared weight license. Whether SlopShop
does the same is a policy decision for the maintainer (§8).

### 3.5 What other editors do

- **Photoshop.** Select Subject offers *Device* or *Cloud (detailed results)*. Cloud processing
  was updated in April 2025. An improved on-device model, reported to be close to the cloud
  on hair, was in beta in September 2025 and later shipped
  ([PhotoshopCAFE](https://photoshopcafe.com/on-device-select-subject-in-photoshop-improved-ai-selections-without-the-cloud/),
  [Glyn Dewis](https://glyndewis.com/blog/photoshop-cloud-selections)).
  - Adobe's rationale: device models are limited by footprint size, cloud models are not.
  - The Object Selection tool has an *Object Finder* that pre-detects objects and highlights
    them on hover. Select Sky is a dedicated command.
  - Select and Mask has "Refine Hair" and an "Object Aware" refine mode (from memory,
    unverified). It is a separate refinement step after the coarse selection, which is the
    two-stage structure proposed here.
  - Internals (models, working resolution) are not public. A forum report shows Select Sky
    failing once on a 12,000 px panorama and then succeeding, which tells nothing about how
    it handles large files.
  - In June 2026 Adobe moved the Remove tool on-device
    ([PhotoWorkout](https://www.photoworkout.com/adobe-june-2026-on-device-ai-photoshop-lightroom/)).
- **Lightroom Classic.** AI masks are stored as data in the catalog (`.lrcat-data`). When an
  edit *may* affect them, they are flagged "Update AI Settings" instead of being recomputed
  silently, and the user triggers the update, also in batch
  ([Adobe community](https://community.adobe.com/t5/lightroom-classic-discussions/p-ai-masks-require-updating-and-or-disappear/td-p/15347415)).
  **This is exactly our invalidation model**, and a proven UX.
- **Affinity Photo 2.6 (Feb 2025).** ML Object Selection, with edge matting, and ML Select
  Subject, recordable in macros. Everything runs on device. Models are **optional downloads**
  from Settings › Machine Learning Models: a "Segmentation" model, much larger, and a
  "Saliency" model ([Serif](https://support.serif.com/hc/en-us/articles/12017241295119-Affinity-Photo-v2-6-includes-Machine-Learning-Features),
  [ProVideo Coalition](https://www.provideocoalition.com/affinity-2-6-introduces-machine-learning-tools-that-you-control/)).
  The runtime and model sizes are not public (support page returned 403).
- **Krita.** No built-in AI selection in 5.3/6.0 (Feb 2026). Plugins:
  - [Krita Vision Tools](https://github.com/Acly/krita-vision-tools): vision.cpp/ggml on
    Vulkan, MobileSAM by default (point and box), a **"Precise" mode with BiRefNet**, and
    BiRefNet background removal. GGUF models, alternatives downloadable; Windows and Linux.
  - Smart Segments (SAM 2).
  - SAM Select (SAM 3 on Apple MLX).
- **GIMP 3.** No built-in AI selection. Python plugins run SAM 2 through ONNX (installing their
  own Python environment and fetching models on first use) or SAM 3/SAM 2 live
  ([gimp_sam2_segmentation](https://github.com/mamipi972/gimp_sam2_segmentation),
  [gimp-sam-select](https://github.com/bunnywaffle/gimp-sam-select)). There is also a
  selection-refiner plugin.

Pattern across editors: a light model for interactive clicks, a heavier one for
"precise"/subject, a separate refinement stage, optional model downloads, and on-device by
default.

### 3.6 Published latencies

| Model | Hardware / runtime | Latency |
|---|---|---|
| MobileSAM encoder 1024² | RTX 4070, vision.cpp Vulkan fp16 / PyTorch fp16 | 19 ms / 16 ms |
| BiRefNet 1024² | RTX 4090 PyTorch fp16 | 57.7 ms |
| BiRefNet 1024² | RTX 4070 vision.cpp Vulkan fp16 / PyTorch | 208 ms / 190 ms |
| BiRefNet_lite 1024² | RTX 4070 vision.cpp Vulkan fp16 | 85 ms |
| BiRefNet 1024² | RTX 4080S TensorRT / PyTorch | 110 ms / 150 ms |
| SAM 2.1 T / L (video frame) | A100 PyTorch | 11 ms / 25 ms |
| SAM 3 encoder 1008² | RTX 3090 PyTorch bf16 / f32 | 110 ms / 390 ms |
| SAM 3 text detection | RTX 3090 PyTorch bf16 | 22 ms |
| Grounding DINO T 800×1333 | A100 fp16 | 112 ms |

Sources: [vision.cpp](https://github.com/Acly/vision.cpp), [BiRefNet](https://github.com/ZhengPeng7/BiRefNet), [SAM 2](https://github.com/facebookresearch/sam2), [sam3 #424](https://github.com/facebookresearch/sam3/issues/424), [Roboflow](https://playground.roboflow.com/models/idea-research/grounding-dino).

---

## 4. Huge images (100–500 MP)

Every model above works at a fixed or bounded resolution (≈1024, up to ~2300 px for
BiRefNet_dynamic). A 20,000 × 15,000 image at 1024 px long side is ~20× downscaled; SAM's
256² logits are then ~78 document pixels per logit. Running the model "on the full image" is
neither possible nor meaningful. The proposed pipeline reuses the document pyramid
(ADR 0022: power-of-two levels, content-hashed tiles):

1. **Model input is an explicit, named conversion.** Take the chosen source (current layer or
   the composite, a product choice). Convert it to the model's expected encoding
   (sRGB-encoded, [0, 1], normalized by the model's mean and std, with a defined tone map for
   HDR/scene-linear data). This is a *view*, not a document change.
2. **Global pass.** Pick the pyramid level whose long side is just ≥ the model input, resize to
   the model size, and run the encoder. Cache the embedding keyed by (source content hash,
   level, ROI, model id + version + precision + EP). A click then costs only the decoder.
3. **ROI pass (zoom-aware).** When the user is zoomed in or draws a box, encode the visible
   region or box plus a context margin at the model resolution. The effective resolution is
   then the screen's, which is what the user is judging. This mirrors approach A of
   hd-generative-ai.md. Points and boxes are stored in document coordinates, so the same prompt
   can be re-run on another ROI.
4. **Coarse mask → full resolution, boundary band only.**
   - Threshold or keep the soft logits.
   - Derive the uncertain band: |logit| < τ, dilated by a few model pixels.
   - Tiles fully inside or outside the band are constant (0 or 65535) and are not computed.
   - Band tiles are refined at full resolution, tile by tile, in wgpu:
     - (a) edge-aware upsampling (guided filter / joint bilateral) with the full-resolution
       luminance as guide;
     - (b) optionally, learned refinement on band tiles: CascadePSP, BiRefNet-matting at
       1024–2048 per tile with overlap, or ViTMatte with a trimap from the band;
     - (c) optionally, a second SAM decoder pass on a band crop with points sampled from the
       coarse mask (like CascadePSP's "local" step).
5. **Output:** a 16-bit selection mask, tiled. Cost scales with the **perimeter** of the
   object, not the image area. A preview is available after step 2 (coarse mask, upscaled),
   and the final mask arrives progressively, like the display cache's refinement.
6. **Determinism and storage:** fp16 and TensorRT tactic choices are not bit-reproducible
   across GPUs, drivers or EPs. The authoritative result is the **stored mask**. The node
   records model id + version + file hash + EP + precision + prompts + ROI + source revision,
   is marked **stale** when the source changes (Lightroom's "Update AI Settings"), and
   recomputes only on request.

Select Subject on a 300 MP image could run BiRefNet_HR or BiRefNet_dynamic at 2048 on the
global level, then (4) on the band. Text selection can run detection globally, then SAM per
box, with ROI encoding when a box is small relative to the image.

---

## 5. Model distribution

- **Download on first use**, with explicit consent showing the size and license, like
  Affinity. Do not bundle models in the installer: the smallest useful set is about 150–600 MB
  and SAM 3 is about 1.7–3.5 GB.
- Possible tiers (est., fp16 ONNX):

  | Tier | Content | Size |
  |---|---|---|
  | Light | SAM 2.1-T encoder 67 MB + decoder (est. < 20 MB) + BiRefNet_lite 115 MB | ≈ 200 MB |
  | Standard | SAM 2.1-B+ (≈ 165 MB est.) + BiRefNet 490 MB + Grounding DINO-T 360 MB | ≈ 1 GB |
  | Optional | SAM 3 | ≈ 1.7 GB+ |

- **Runtime download too:** with `load-dynamic`, ORT plus the chosen EP (DirectML small,
  TensorRT-RTX about 200 MB) can be downloaded the same way, so the editor install stays lean.
  The editor works without AI.
- **Manifest** (in the app, versioned with it): model id, semantic version, files with
  **upstream-pinned URLs** (Hugging Face `resolve/<commit-sha>/<file>` is immutable), size,
  SHA-256 (HF exposes the LFS SHA-256 of each file, so the hash can be checked against
  upstream), license id + text, input size, opset, precision, supported EPs.
- **Integrity:** HTTP range resume, verify SHA-256, then an atomic rename. Never load an
  unverified file.
- **Cache:** per-user (Windows `%LOCALAPPDATA%\SlopShop\models\<id>\<version>\`), overridable,
  with a "manage models" panel to list, delete and import a file offline.
- **TensorRT-RTX engine cache** is keyed by model hash + GPU + driver + EP version and lives
  next to the models. The first use after a driver update recompiles (seconds; to be measured).
- **Hosting:** upstream Hugging Face for permissive models. Our own ONNX exports (fp16, opset
  22, fixed shapes) need a home of our own, such as an HF organization or GitHub Releases
  (2 GB per file limit). Gated models (SAM 3) need either the user's HF token or our own
  mirror; the SAM License allows redistribution with the license.
- **Documents** store model id + version + file hash in the node provenance, never the model.
  Opening a document without the model shows the stored mask and offers the download only for
  a re-run.

---

## 6. Recommended stack (to confirm by experiment)

| Feature | Model (default → alternatives) | Notes |
|---|---|---|
| Runtime | `ort` 2.0 (ORT 1.30), `load-dynamic`, in a **`slopshop-ai` helper process** | EP order. Windows: TensorRT-RTX → DirectML → CPU. Linux: CUDA or TensorRT-RTX → (WebGPU) → CPU. macOS: CoreML → CPU. Evaluate WebGPU EP as the cross-vendor path. |
| Click / box | **SAM 2.1** Base+ (Small or Tiny on low VRAM) → HQ-SAM 2 if edges are clearly better → SlimSAM/MobileSAM for CPU | Encoder cached per (source, ROI); decoder per click. Multimask output for ambiguity. |
| Select Subject | **BiRefNet_dynamic or BiRefNet_HR** (2048) → BiRefNet_lite → BEN2 to compare | fp16, opset ≥ 22. |
| Text / semantic | **Grounding DINO-T + SAM 2.1** (permissive). **Florence-2-B** for referring expressions. **SAM 3 as an optional download** if the maintainer accepts the SAM License. | Check "stuff" prompts (sky, water, grass) specifically. |
| Edge refinement | **Classical edge-aware upsampling on the boundary band** (wgpu, full resolution) → **BiRefNet(-HR)-matting** or **CascadePSP** on band tiles | ViTMatte only if its provenance is accepted. |

---

## 7. Experiments (benchmark harness)

1. **Exports** (in `experiments/ai-selection/`, Python allowed). For each model: ONNX fp32 and
   fp16, opset 22, static shapes where possible, plus PyTorch reference outputs on the eval set.
   Export as ONNX ourselves the models that have no export: HQ-SAM 2, CascadePSP and BiRefNet
   at opset 22.
2. **Latency harness** (Rust, `slopshop-ai bench`, using `ort`). For each model × EP ×
   precision × input size, record:
   - session creation time;
   - first-run time (including TensorRT-RTX JIT) and the effect of the engine cache;
   - warm p50/p95 over 50 runs;
   - peak VRAM (DXGI `QueryVideoMemoryInfo` or NVML) and host RAM;
   - **per-node EP placement** (ORT verbose log or profiling; any CPU fallback is a failure to
     investigate);
   - output agreement with PyTorch (mask IoU, max |Δlogit|).

   The matrix:

   | Platform | EPs |
   |---|---|
   | Windows / RTX 5070 Ti (maintainer) | TensorRT-RTX, DirectML, WebGPU, CUDA (reference), CPU |
   | Windows / AMD or Intel GPU (if anyone has one) | DirectML, WebGPU |
   | macOS CI | CoreML, CPU |
   | Linux CI | CPU, WebGPU (CI may have no GPU) |

3. **Quality set.** 50–100 images with licenses that allow it (the maintainer's own photos,
   CC0), several at 24–100+ MP. Hand-labelled 16-bit masks covering hair and fur, transparent
   and fine objects (bicycles, branches), sky with trees, people in groups, cars, tiny objects,
   low contrast and backlight. Plus 3–5 text prompts per image, including "stuff" and
   attribute phrases.
   - Metrics: IoU; boundary F-score / boundary IoU **at full resolution**; for hair crops,
     alpha SAD/MSE/gradient error; click efficiency NoC@85/90 with simulated clicks; text
     prompts precision/recall per prompt.
4. **Large-image pipeline.** On 100 MP and 300 MP images, compare:
   - global only;
   - global + ROI;
   - global + band refinement (classical);
   - global + band refinement (learned).

   Measure time to first preview, time to final mask, full-resolution boundary F, and peak
   VRAM/RAM.
5. **Proposed interactive targets** (for the maintainer to confirm):
   - click → mask < 50 ms once the embedding is cached;
   - embedding < 300 ms;
   - Select Subject preview < 1 s;
   - final 100 MP mask < 3 s.
6. **Helper process overhead.** Round-trip of a 2048² input and 1024² logits through the pipe,
   process start time, model load time.
7. **Decision gate.** An ADR records runtime, process model, models per feature, the weight
   license policy and the distribution scheme, with the measured evidence.

---

## 7b. First measurements (2026-10-01)

`experiments/ai-bench` (ONNX Runtime 1.28 through `ort` 2.0.0-rc.13, synthetic inputs of the
real shapes, models pinned in `models.tsv`). Maintainer's machine: RTX 5070 Ti (16 GB), driver
595.97, Intel Core Ultra 7 265KF, Windows 11. Warm median (p50) per run; the first run of a
session adds 0.2–0.9 s, creating a session 0.25–2.8 s. VRAM is the growth of the GPU's memory in
use (nvidia-smi), so approximate.

| Model | Stage | DirectML p50 | VRAM | CPU p50 |
|---|---|---|---|---|
| SAM 2.1 tiny fp32 | image encoder | 34 ms | 2.0 GB | 425 ms |
| SAM 2.1 tiny fp16 | image encoder | 21 ms | 0.9 GB | 718 ms |
| SAM 2.1 small fp16 | image encoder | 25 ms | 0.9 GB | 847 ms |
| SAM 2.1 base+ fp16 | image encoder | 40 ms | 1.2 GB | 1367 ms |
| SAM 2.1 (any) | decoder, one click | 4–6 ms | 0.2 GB | 14–21 ms |
| BiRefNet lite fp16 | 1024² | 91 ms | **11 GB** | 2.8 s |
| BiRefNet fp16 | 1024² | 144 ms | **10 GB** | 5.3 s |
| BiRefNet dynamic fp16 | — | **export broken** (type error at load) | | |
| SAM 3 | image encoder | **fails on DirectML** (E_INVALIDARG in graph init, at every optimization level) | | 4.9 s |
| SAM 3 | text encoder | 17 ms | 1.6 GB | 31 ms |
| SAM 3 | decoder | 76 ms | 2.2 GB | 714 ms |
| Grounding DINO tiny fp16 | text → boxes | **fails on DirectML** (same error) | | not measured (synthetic token ids out of range) |

Readings:
- **Click and box (SAM 2.1) are settled**: interactive even on the CPU once the image is
  encoded; on the GPU the whole loop is a few tens of milliseconds for about 1 GB.
- **Select Subject (BiRefNet) is fast but uses 10–11 GB of VRAM on DirectML** with these exports,
  where deformable convolution is unrolled into GatherND/ScatterND chains: an export with native
  `DeformConv` (BiRefNet PR #167) or another EP is needed before it can ship.
- **Text selection does not run on DirectML** (SAM 3's image encoder, Grounding DINO): it needs
  TensorRT-RTX or CUDA on NVIDIA (the maintainer's choice), and WebGPU is to be tried for other
  GPUs; on the CPU, SAM 3 takes about 5 s per image.
- Next: TensorRT-RTX (NVIDIA libraries downloaded on demand), WebGPU, the native-DeformConv
  BiRefNet export, Grounding DINO with real tokens, then the large-image pipeline (coarse mask
  plus refinement of the uncertain band at full resolution) and quality on labelled images.

### Other execution providers (2026-10-01)

- **WebGPU**: `ort`'s prebuilt ONNX Runtime with WebGPU does not link with MSVC 14.44 (Visual
  Studio 2022 Build Tools): it needs a newer C++ standard library (`__std_rotate`,
  `__std_max_element_8i`…, Visual Studio 2026). Not measured yet; installing newer build tools
  is the maintainer's call. In the product the helper would ship as a built executable, so this
  only concerns building it.
- **TensorRT for RTX**: downloading the SDK needs an **NVIDIA Developer Program account** and
  accepting NVIDIA's license; NVIDIA lists the **CUDA Toolkit 12.9 or later** as a prerequisite;
  RTX 30 series and later. ONNX Runtime's built-in TensorRT-RTX EP is deprecated in favour of
  NVIDIA's standalone plugin (NVIDIA/TensorRT-RTX-EP-ABI), which is built from source today
  ([ONNX Runtime](https://onnxruntime.ai/docs/execution-providers/TensorRTRTX-ExecutionProvider.html),
  [NVIDIA](https://docs.nvidia.com/deeplearning/tensorrt-rtx/latest/installing-tensorrt-rtx/installing.html)).
  How SlopShop could fetch it for its users (redistribution terms, account) is for the ADR.

### NVIDIA execution providers (2026-10-01)

Setup without any account: an isolated Python environment holding NVIDIA's wheels from PyPI
(`nvidia-cuda-runtime` with cuBLAS, `nvidia-cudnn-cu13`, `tensorrt_rtx_cu13_libs` 1.6), no
system CUDA install.
- **`ort`'s prebuilt CUDA build has no kernels for the RTX 50 series** (Blackwell, sm_120):
  "no kernel image is available for execution on the device".
- **`ort`'s prebuilt TensorRT-RTX provider is linked against `nvinfer_10.dll`**, while NVIDIA's
  current TensorRT for RTX (1.6) ships `tensorrt_rtx_1_6.dll`; loading 1.6 under the old name
  crashes (ABI). TensorRT-RTX therefore needs NVIDIA's standalone plugin EP, built from source.
- **Microsoft's official ONNX Runtime 1.28 CUDA 13 build works** (365 MB zip, MIT, loaded at run
  time with `load-dynamic`). Warm p50 on the RTX 5070 Ti:

| Model | Stage | CUDA p50 (p95) | VRAM growth |
|---|---|---|---|
| SAM 2.1 tiny fp32 / small fp16 / base+ fp16 | image encoder | 22 / 17 / 30 ms | 1.3–2.4 GB |
| SAM 2.1 | decoder, one click | 3.6–3.7 ms | 0.26 GB |
| BiRefNet lite / full fp16 | 1024² | 120 / 200 ms | 7.5 / 8.5 GB |
| SAM 3 | image encoder | 224 ms (733) | 12.9 GB |
| SAM 3 | text encoder | 5.6 ms | 2.2 GB |
| SAM 3 | decoder | 59 ms | 5.8 GB |
| Grounding DINO tiny fp16 | text → boxes | not measured (synthetic inputs rejected; real tokens needed) | |

- **SAM 3's text selection runs on the GPU**: about 0.3 s for a new image, then 65 ms per prompt.
  Its memory growth (and BiRefNet's) includes ONNX Runtime's arena, which grows by powers of two;
  `arena_extend_strategy = kSameAsRequested` and fp16 exports are to be tried before judging.
- The product would ship Microsoft's build in the `slopshop-ai` helper, with NVIDIA's runtime
  libraries fetched on demand (cuDNN alone is about 900 MB unpacked).

### Large images: coarse mask, then refinement of the band (2026-10-01)

The maintainer's idea, prototyped in `ai-bench refine` on a CC0 portrait with loose grey hair on
a blurred grey background (4000×5000, [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Bearded_man_with_long_hair-3052641.jpg)),
CUDA, warm:
1. downscale to 1024² (66 ms on the CPU, to move to the GPU), SAM 2.1 base+ with a box: encoder
   44 ms, decoder about 5 ms;
2. logits upsampled to full resolution: 16 ms; the coarse edge is a blur about 20 px wide (the
   decoder's 256² grid), and SAM's grid shows as faint blocks on textured clothes;
3. uncertain band (1–99 %): 2–46 % of the pixels depending on the subject (a threshold on the
   probability is too wide; a distance to the half-way contour is better);
4. refinement of the band:
   - **classical guided filter** (luminance guide, tiles with margins): 70–180 ms for 20 MP; it
     tightens the edge but **does not recover strands** when hair and background have similar
     tones (radius 8 to 32 tried);
   - **BiRefNet lite on a full-resolution crop of the band** (800² resized to its 1024² input):
     **161 ms per crop**, and it **separates individual strands**, background showing between
     them: real hair matting.

Conclusion: coarse selection by SAM (instant), then **learned refinement on the band's tiles at
full resolution** (progressive, about 0.16 s per tile on this GPU), the classical filter only as
a fallback without a GPU. Still to measure: seams between overlapping tiles, the cost on
100–300 MP, BiRefNet's VRAM, and a comparison with dedicated matting models.

### Refinement when the selection is not "the subject" (2026-10-01)

The maintainer's question: BiRefNet decides by itself what the foreground is. Tested on a 50 MP
CC0 photo of a long-haired cat on a wooden fence ([Wikimedia Commons](https://commons.wikimedia.org/wiki/File:Long-haired_calico_cat_on_top_of_wooden_fence_2025-09-21.jpg)):
- **cat selected** (one click): BiRefNet on a full-resolution crop of the band separates the
  fur hair by hair against the sky (one small hole in a dark patch); the guided filter leaves a
  halo of sky and cuts the hairs;
- **fence selected** (one click on the rail): BiRefNet on a crop where the fur hangs over the
  rail mattes **the cat**, the opposite of the selection there.

Guard prototyped: BiRefNet's crop is compared with the coarse mask where that one is sure (below
1 % or above 99 %): used as it is when it agrees (error 0.03 for the cat), inverted when it is the
exact opposite, and otherwise the guided filter (error 0.43 / 0.57 for the fence: BiRefNet
picked a third region), and only inside the uncertain band. This avoids gross errors but gives
no hair detail when the selection is not salient: a **mask- or trimap-guided, class-agnostic
matting model** is the principled refinement (candidates and licenses under review).

### Mask-guided matting: ViTMatte-S (2026-10-01)

Candidates reviewed for a class-agnostic refinement guided by the selection: ViTMatte (code MIT,
weights Apache-2.0, ONNX `Xenova/vitmatte-small-composition-1k`, 104 MB), MEMatte (MIT, no
ONNX), Matting Anything (tied to SAM 1), CascadePSP / SegRefiner (binary output, poor on fur),
DiffMatte (10 diffusion steps); excluded as non-commercial: MGMatting, ZIM, SAM2Matting, Matte
Anything, MatAnyone. **Caveat for the maintainer: almost every matting model, ViTMatte included,
is trained on Adobe Composition-1k or Distinctions-646, whose terms are research-only; whether
that reaches the published weights is legally unsettled.**

ViTMatte-S on the cat photo, a trimap made from the coarse SAM mask (sure inside, sure outside,
and a band to decide), 1024² per crop: **52 ms per crop** on CUDA (BiRefNet: 160 ms).
- **Fence selected**: correct (BiRefNet matted the cat): the rail is kept, the cat excluded, the
  edge follows the fur; hairs lying over the rail stay with the rail.
- **Cat selected**: close to BiRefNet when the undecided band is wide enough for the long hairs;
  a narrow band cuts them and leaves a light halo.
- **The trimap decides everything**: thresholds on the coarse probability leave no sure region
  where the coarse mask is soft (the model then picks the foreground itself, wrongly); a band of
  fixed width around the half-way contour is safe but cuts long hairs. The band should follow
  the coarse mask's uncertainty (wider in fur): to tune.

Direction: **SAM 2.1 (coarse, any object) → trimap from its mask → ViTMatte on the band's tiles
(class-agnostic, 52 ms per tile)**, with BiRefNet as an option for "Select Subject" where it is
sharper. Before the ADR: band width, seams between tiles, cost on 100+ MP, and the training-data
license question.

## 8. Open questions for the maintainer

1. **Weight license policy** (extends ADR 0006 to model weights):
   - only OSI-permissive declared weight licenses (MIT/Apache/BSD)?
   - Is the **SAM License** (commercial OK, use restrictions, not OSI) acceptable for an
     *optional* SAM 3 download?
   - Do we rely on declared weight licenses regardless of training-data provenance, as other
     editors and plugins do?
2. **Proprietary GPU runtimes:** may SlopShop download NVIDIA's TensorRT-RTX (NVIDIA SLA) on
   demand? Or should it stay on DirectML/WebGPU only, with Windows ML managing vendor EPs at
   the cost of Windows 11 24H2+?
3. **Process model:** a persistent `slopshop-ai` helper (recommended), or `ort` in-process
   behind a feature?
4. **Distribution:** first-use download from upstream HF vs. our own mirror; tiers (light,
   standard, optional SAM 3); offline import.
5. **Minimum hardware and CPU fallback:** is CPU-only AI selection offered (slow, seconds)? Is
   there a VRAM floor?
6. **Product:** sample the current layer or all layers? Hover-to-highlight "object finder"
   (needs a full-image automatic segmentation up front, costly)? Is a separate "Refine edge /
   hair" step acceptable, or must it be automatic?
7. **Node semantics for selections:** keep the committed mask plus provenance with a stale
   flag (recommended, as in Lightroom), or a live AI node that re-runs on request?
