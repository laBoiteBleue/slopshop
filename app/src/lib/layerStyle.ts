// Layer styles in the UI (ADR 0032): the effects in Photoshop's order, their defaults, and the
// changes the Layer Style dialog, the layers panel and the Layer menu make to a style. The
// engine draws the effects; these rules only build the style it is sent.

import type { EditRequest, LayerStyle, LayerView } from "./engine";
import type { MessageKey } from "./i18n/en";

/** A color of an effect the color picker sets: most have one; Bevel and Emboss two. */
export type EffectColor = "color" | "highlightColor" | "shadowColor";

/** An effect a style can hold. */
export type EffectId =
  | "bevel"
  | "stroke"
  | "innerShadow"
  | "innerGlow"
  | "satin"
  | "colorOverlay"
  | "gradientOverlay"
  | "patternOverlay"
  | "outerGlow"
  | "dropShadow";

/** In the order of Photoshop's Layer Style dialog and layers panel (topmost drawn first). */
export const EFFECTS: { id: EffectId; label: MessageKey }[] = [
  { id: "bevel", label: "style.bevel" },
  { id: "stroke", label: "style.stroke" },
  { id: "innerShadow", label: "style.innerShadow" },
  { id: "innerGlow", label: "style.innerGlow" },
  { id: "satin", label: "style.satin" },
  { id: "colorOverlay", label: "style.colorOverlay" },
  { id: "gradientOverlay", label: "style.gradientOverlay" },
  { id: "patternOverlay", label: "style.patternOverlay" },
  { id: "outerGlow", label: "style.outerGlow" },
  { id: "dropShadow", label: "style.dropShadow" },
];

/** No effect, Fill Opacity at 100 %: what a layer without a style shows. */
export const PLAIN: LayerStyle = {
  fillOpacity: 1,
  dropShadow: null,
  outerGlow: null,
  innerShadow: null,
  innerGlow: null,
  colorOverlay: null,
  gradientOverlay: null,
  patternOverlay: null,
  satin: null,
  stroke: null,
  bevel: null,
};

/** The effects added at their defaults: a Pattern Overlay needs its pattern chosen first. */
export type DefaultEffectId = Exclude<EffectId, "patternOverlay">;

/** An effect as Photoshop adds it (as `style.rs`'s defaults), enabled. */
export function defaultEffect<E extends DefaultEffectId>(id: E): NonNullable<LayerStyle[E]> {
  const shadow = {
    enabled: true,
    color: [0, 0, 0],
    mode: "multiply",
    opacity: 0.75,
    angle: 120,
    distance: 5,
    spread: 0,
    size: 5,
  } as const;
  // Pale yellow, sRGB #ffffbe.
  const glow = {
    enabled: true,
    color: [1, 1, 190 / 255],
    mode: "screen",
    opacity: 0.75,
    spread: 0,
    size: 5,
  } as const;
  const effects = {
    dropShadow: { ...shadow, color: [0, 0, 0] },
    innerShadow: { ...shadow, color: [0, 0, 0] },
    outerGlow: { ...glow, color: [1, 1, 190 / 255] },
    innerGlow: { ...glow, color: [1, 1, 190 / 255] },
    colorOverlay: { enabled: true, color: [1, 0, 0], mode: "normal", opacity: 1 },
    bevel: {
      enabled: true,
      style: "innerBevel",
      depth: 100,
      up: true,
      size: 5,
      soften: 0,
      angle: 120,
      altitude: 30,
      highlightColor: [1, 1, 1],
      highlightMode: "screen",
      highlightOpacity: 0.75,
      shadowColor: [0, 0, 0],
      shadowMode: "multiply",
      shadowOpacity: 0.75,
    },
    satin: {
      enabled: true,
      color: [0, 0, 0],
      mode: "multiply",
      opacity: 0.5,
      angle: 19,
      distance: 11,
      size: 14,
      invert: true,
    },
    // Black to white from bottom to top, aligned with the layer.
    gradientOverlay: {
      enabled: true,
      stops: [
        [0, 0, 0, 0],
        [4096, 255, 255, 255],
      ],
      reverse: false,
      shape: "linear",
      angle: 90,
      scale: 100,
      align: true,
      mode: "normal",
      opacity: 1,
    },
    stroke: {
      enabled: true,
      size: 3,
      position: "outside",
      color: [0, 0, 0],
      mode: "normal",
      opacity: 1,
    },
  } satisfies { [K in DefaultEffectId]: NonNullable<LayerStyle[K]> };
  return structuredClone(effects[id]) as NonNullable<LayerStyle[E]>;
}

/** `style`, or nothing when it changes nothing (no effect, Fill at 100 %). */
export function simplified(style: LayerStyle): LayerStyle | null {
  const none = EFFECTS.every(({ id }) => !style[id]);
  return none && style.fillOpacity >= 1 ? null : style;
}

/**
 * `style` (or none) with effect `id` on or off: added at its defaults when it was not there,
 * kept with its settings when turned off (as Photoshop's checkboxes and eyes).
 */
export function withEffect(style: LayerStyle | null, id: EffectId, enabled: boolean): LayerStyle {
  const base = style ?? PLAIN;
  const current = base[id];
  // A Pattern Overlay is added with its pattern chosen (`withPatternOverlay`).
  if (!current && id === "patternOverlay") return base;
  const effect = current ?? defaultEffect(id as DefaultEffectId);
  return { ...base, [id]: { ...effect, enabled } };
}

/**
 * `style` (or none) with a Pattern Overlay of source `source`, on: its other settings kept, or
 * Photoshop's (100 %, upright, linked to the layer, Normal, opaque) when it had none.
 */
export function withPatternOverlay(style: LayerStyle | null, source: number): LayerStyle {
  const base = style ?? PLAIN;
  const overlay = base.patternOverlay ?? {
    enabled: true,
    source,
    scale: 1,
    angle: 0,
    link: true,
    mode: "normal" as const,
    opacity: 1,
  };
  return { ...base, patternOverlay: { ...overlay, source, enabled: true } };
}

/** `style` (or none) with Fill Opacity `fill` in [0, 1]; nothing when that leaves it plain. */
export function withFill(style: LayerStyle | null, fill: number): LayerStyle | null {
  return simplified({ ...(style ?? PLAIN), fillOpacity: fill });
}

/** The effects `style` holds, in the panel's order, whether each is on. */
export function effectsOf(
  style: LayerStyle | null | undefined,
): { id: EffectId; enabled: boolean }[] {
  if (!style) return [];
  return EFFECTS.flatMap(({ id }) => {
    const effect = style[id];
    return effect ? [{ id, enabled: effect.enabled }] : [];
  });
}

/** `style` without effect `id` (deleted, its settings with it); nothing when that leaves it plain. */
export function withoutEffect(style: LayerStyle | null, id: EffectId): LayerStyle | null {
  return simplified({ ...(style ?? PLAIN), [id]: null });
}

/**
 * Whether Fill means something for `style`, so that the layers panel shows it: an effect, even
 * turned off (Fill fades the content and leaves it), or a Fill already set. Without either, Fill
 * would do what Opacity does; Blending Options still sets it.
 */
export function usesFill(style: LayerStyle | null | undefined): boolean {
  return effectsOf(style).length > 0 || (style?.fillOpacity ?? 1) < 1;
}

/** Whether `style` has an effect turned on (the panel's fx mark). */
export function hasEffects(style: LayerStyle | null | undefined): boolean {
  return effectsOf(style).some((e) => e.enabled);
}

/** Fill Opacity `fill` for `layers` but adjustment layers (one undo entry). */
export function fillEdit(layers: LayerView[], fill: number): EditRequest | null {
  const edits = layers
    .filter((l) => l.kind !== "adjustment")
    .map((l) => styleEdit(l.id, withFill(l.style ?? null, fill)));
  if (edits.length === 0) return null;
  return edits.length === 1 ? edits[0] : { kind: "batch", edits };
}

/** The edit that gives layer `id` `style` (none: removed). */
export function styleEdit(id: number, style: LayerStyle | null): EditRequest {
  return { kind: "setLayerStyle", id, style: style && simplified(style) };
}
