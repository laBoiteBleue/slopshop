// Layer styles in the UI (ADR 0032): the effects in Photoshop's order, their defaults, and the
// changes the Layer Style dialog, the layers panel and the Layer menu make to a style. The
// engine draws the effects; these rules only build the style it is sent.

import type { EditRequest, LayerStyle, LayerView } from "./engine";
import type { MessageKey } from "./i18n/en";

/** An effect a style can hold. */
export type EffectId =
  "stroke" | "innerShadow" | "innerGlow" | "colorOverlay" | "outerGlow" | "dropShadow";

/** In the order of Photoshop's Layer Style dialog and layers panel (topmost drawn first). */
export const EFFECTS: { id: EffectId; label: MessageKey }[] = [
  { id: "stroke", label: "style.stroke" },
  { id: "innerShadow", label: "style.innerShadow" },
  { id: "innerGlow", label: "style.innerGlow" },
  { id: "colorOverlay", label: "style.colorOverlay" },
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
  stroke: null,
};

/** An effect as Photoshop adds it (as `style.rs`'s defaults), enabled. */
export function defaultEffect<E extends EffectId>(id: E): NonNullable<LayerStyle[E]> {
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
    stroke: {
      enabled: true,
      size: 3,
      position: "outside",
      color: [0, 0, 0],
      mode: "normal",
      opacity: 1,
    },
  } satisfies { [K in EffectId]: NonNullable<LayerStyle[K]> };
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
  const effect = base[id] ?? defaultEffect(id);
  return { ...base, [id]: { ...effect, enabled } };
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
