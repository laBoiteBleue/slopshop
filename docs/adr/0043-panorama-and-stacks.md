# 0043 — Panoramas, aligned layers and image stacks

Status: accepted (2026-10-07, the maintainer's answers below).

## Context

Photoshop combines several photos four ways:

- **File > Automate > Photomerge**: a panorama from files, layouts Auto, Perspective,
  Cylindrical, Spherical, Collage (rotate and scale) and Reposition (move only), with Blend
  Images Together, Vignette Removal, Geometric Distortion Correction and Content-Aware Fill
  Transparent Areas.
- **Edit > Auto-Align Layers**: the same alignment on layers already in a document.
- **Edit > Auto-Blend Layers**: Panorama (seams between overlapping layers) or Stack Images
  (focus stacking: the sharpest parts of each), with Seamless Tones and Colors. It makes a layer
  mask per layer.
- **Smart Object Stack Modes**: a per-pixel statistic over aligned layers (Median takes out
  passers-by, Mean lowers noise; also Minimum, Maximum, Range, Sum, Variance, Standard
  Deviation, Skewness, Kurtosis, Entropy), over non-transparent pixels.

SlopShop has what most of this needs: projective layer transforms
([ADR 0038](0038-projective-transforms.md)), layer masks ([ADR 0014](0014-layer-masks.md)),
adjustments as stack entries ([ADR 0034](0034-editable-operations.md)), a min-cut solver
(`maxflow.rs`, Quick Selection), blurs and image pyramids, Import as Layers (folders too), and
sources ([ADR 0040](0040-sources.md)), which plans a *stack* source kind combining several.

A survey (2026-10-07) found no maintained Rust stitching library. Feature detection exists:
`kornia-imgproc` (Apache-2.0, active, SIFT ported from OpenCV and ORB, but pre-1.0 and with
`unsafe` SIMD), `lowe-sift` (MIT, safe, unoptimized). The rest (robust estimation, bundle
adjustment, cylindrical and spherical maps, seams, exposure gains, multi-band blending, focus
measures) is a few thousand lines following published methods (Brown and Lowe 2007, Burt and
Adelson 1983, Pertuz et al. 2013) and OpenCV's stitching module (Apache-2.0) as a reference;
Hugin and enblend are GPL, ideas only.

## Decision

1. **Alignment is layer transforms.** Auto-Align (and Photomerge's first step) finds features on
   a reduced level of each layer (2 to 4 megapixels), matches them, estimates each pair's map
   robustly (RANSAC), then all of them together (bundle adjustment), and sets each layer's
   transform: a translation (Reposition), a similarity (Collage) or a homography (Perspective,
   ADR 0038). Nothing is resampled: the transforms stay editable with Free Transform, one undo
   entry for all.
2. **Cylindrical and Spherical** (wide panoramas, which no homography can hold) and lens
   distortion are not projective: they exist only inside a panorama source (a stack source
   placing each image through its projection), not as a new kind of layer transform.
3. **Blending is layer masks**, as Photoshop's: Auto-Blend Panorama computes seams between
   overlapping layers (a minimum cut on the reduced overlap, refined in a band at full
   resolution) and gives each layer a mask; Stack Images gives each layer a mask where it is
   the sharpest (a smoothed Laplacian energy). Seamless Tones and Colors adds an exposure gain
   entry to each layer's stack; Vignette Removal a radial gain entry. Every result stays
   editable: masks are painted, entries edited.
4. **Multi-band blending** (seams invisible across exposure and detail differences) cannot be
   expressed as alpha: hard and feathered masks first; later, a stack source blending the
   layers by their masks, tile by tile.
5. **Stack Modes are a stack source** (ADR 0040, point 1): a source reading several image
   sources through their transforms and computing a per-pixel statistic, tile by tile, cached
   like the other computed sources, CPU first then GPU. Layer > Smart Objects > Stack Mode
   becomes Layer > Combine Layers > (Median, Mean…) on the selected aligned layers, the result a
   layer showing that source; changing the mode makes a new source.
6. **Photomerge** opens the chosen files as layers of a new document (Import as Layers), aligns
   them (point 1), blends them (point 3), and groups the layers.
7. **The code** lives in a new crate, `slopshop-align` (features, matching, estimation),
   depending on core; seams, masks and stack sources live in core, tested against references.
   Feature detection: SIFT written in-house (no dependency), following Lowe's paper with
   OpenCV's (Apache-2.0) and `lowe-sift`'s (MIT) as references.
8. **Failure is a message, not garbage**: too few matches, a degenerate map (scale or shear out
   of bounds) or parallax leave the layers as they were and say which ones could not be aligned.

## Alternatives

- **Resampled results** (Photoshop's Photomerge output): simple to draw, but nothing could be
  adjusted afterwards; transforms and masks cost nothing to keep.
- **OpenCV through bindings**: complete, but a native C++ dependency and its build (ADR 0006).
- **Exposure fusion and HDR merge in the same work**: related (pyramids, alignment), left for
  later.

## Consequences

- Edit > Auto-Align Layers, Auto-Blend Layers and File > Automate > Photomerge in the menus once
  each works; Layer > Combine Layers with the stack source.
- A stack source kind in the model, `.slop` and the renderer (ADR 0040's caches).
- Content-Aware Fill of the transparent corners waits for the generative and content-aware work.

## Answers (the maintainer, 2026-10-07)

1. Cylindrical, Spherical and lens correction: only inside a panorama source, not a new kind of
   layer transform.
2. Hard and feathered masks first; multi-band blending later.
3. SIFT written in-house, no dependency.
4. Order: Stack Modes (Median and Mean first), then Auto-Align and Auto-Blend, then Photomerge.
5. Content-Aware Fill of the transparent corners: later, with the content-aware or AI work.
6. Photomerge's result grouped.
