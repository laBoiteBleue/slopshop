# ai-bench (experiment)

Latency and VRAM of the candidate AI selection models through ONNX Runtime, per execution
provider. Not part of SlopShop: outside the Cargo workspace, never built by CI. Findings go to
[docs/research/ai-selection.md](../../docs/research/ai-selection.md); decisions to an ADR.

```sh
python fetch_models.py            # 5.5 GB of pinned models, SHA-256 checked, no account needed
cargo run --release -- --ep directml --ep cpu [--only sam2.1-tiny] [--runs 20] [--opt basic]
```

Models go to `%LOCALAPPDATA%/slopshop/bench-models` (or `~/.cache/slopshop/bench-models`);
delete that folder to free the space. Licenses: SAM 2.1, Grounding DINO Apache-2.0; BiRefNet
MIT; SAM 3 under the SAM License (its LICENSE file is fetched with it).
