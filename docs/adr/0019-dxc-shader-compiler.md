# 0019 — DXC compiles the shaders on Windows

Status: accepted (2026-09-30).

## Context

On Windows, wgpu turns the WGSL shaders into HLSL and, by default, compiles them with FXC,
Direct3D's old compiler. The compositing shader takes FXC about 11 s (7 s before the resampling
of ADR 0018): the app waits that long for its first image, and every GPU test pays it. FXC also
miscompiled the shader for WARP, the software adapter of Windows CI, when it had several copies
of the resampling loop. DXC, Microsoft's current compiler, takes about 1 s and compiles the same
shader correctly.

## Decision

wgpu's `static-dxc` feature: DXC is linked into the executable (Windows x64 with MSVC), and
wgpu's default compiler choice (`Auto`) uses it. `WGPU_DX12_COMPILER` can still select FXC or a
DXC DLL. Other platforms and Windows on ARM are unchanged (Metal, Vulkan; FXC on ARM).

Building on Windows needs Visual Studio's C++ ATL component (DXC links against ATL); GitHub's
Windows runners have it.

## Alternatives

- **FXC** (the default): nothing to add, but about 11 s at every start, and its WARP bug.
- **DXC as a DLL** (`dxcompiler.dll` from Microsoft's releases, shipped next to the app): no
  ATL, but a file to download at build time, bundle and keep with the executable, and a fall back
  to FXC when it is missing.
- **Vulkan on Windows**: fast drivers' compilers, but DX12's flip-model swapchains are what
  direct presentation relies on (ADR 0002).

## Consequences

- Dependencies: `mach-dxcompiler-rs`, whose build downloads prebuilt DXC static libraries (the
  mach engine project's builds of DXC, permissively licensed).
- The executable grows by a few MB on Windows.
- Contributors on Windows install the ATL component once (README prerequisites).
