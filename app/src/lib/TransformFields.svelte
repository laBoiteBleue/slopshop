<script lang="ts">
  // Free Transform's numbers in the options bar, as in Photoshop: X and Y of the reference
  // point, W and H in % (linked or not), the angle and the horizontal skew in degrees. Each has
  // a slider and a scrubby label, like every numeric setting; a value changes the box live.
  import type { Matrix } from "./engine";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { keepFocus } from "./platform";
  import SliderField from "./SliderField.svelte";
  import { compose, decompose, type TransformValues } from "./transformValues";

  let {
    matrix,
    pivot,
    canvas,
    onchange,
  }: {
    /** The transform so far (a map of the document's space). */
    matrix: Matrix;
    /** The reference point, in the coordinates of the box as it was. */
    pivot: [number, number];
    /** The document's size: X and Y slide over it and as much around it. */
    canvas: { width: number; height: number };
    onchange: (matrix: Matrix) => void;
  } = $props();

  /** W and H change together, as Photoshop's link. */
  let linked = $state(true);

  const values = $derived(decompose(matrix, pivot));
  const DEGREES = 180 / Math.PI;

  type Field = {
    key: keyof TransformValues;
    label: string;
    unit: string;
    /** Shown value = stored × factor; `min`, `max` and `step` are shown values. */
    factor: number;
    min: number;
    max: number;
    step: number;
  };
  const FIELDS = $derived<Field[]>([
    { key: "x", label: "X", unit: "px", factor: 1, ...around(canvas.width) },
    { key: "y", label: "Y", unit: "px", factor: 1, ...around(canvas.height) },
    {
      key: "width",
      label: t("transform.field.width"),
      unit: "%",
      factor: 100,
      min: -1000,
      max: 1000,
      step: 0.1,
    },
    {
      key: "height",
      label: t("transform.field.height"),
      unit: "%",
      factor: 100,
      min: -1000,
      max: 1000,
      step: 0.1,
    },
    { key: "angle", label: "∠", unit: "°", factor: DEGREES, min: -180, max: 180, step: 0.1 },
    {
      key: "skew",
      label: t("transform.field.skew"),
      unit: "°",
      factor: DEGREES,
      min: -85,
      max: 85,
      step: 0.1,
    },
  ]);

  /** A position's range: the canvas side and as much on each side of it. */
  function around(side: number) {
    return { min: -side, max: 2 * side, step: 1 };
  }

  /** A value set (in stored units): the box follows at once. */
  function set(key: keyof TransformValues, value: number) {
    const next = { ...values, [key]: value };
    // Linked: the other side keeps its share of this one.
    if (linked && (key === "width" || key === "height")) {
      const other = key === "width" ? "height" : "width";
      const before = values[key];
      if (before !== 0) next[other] = values[other] * (value / before);
    }
    // A scale of zero cannot be undone.
    if (next.width === 0 || next.height === 0 || Math.abs(next.skew) >= Math.PI / 2) return;
    onchange(compose(next, pivot));
  }
</script>

{#each FIELDS as field (field.key)}
  <SliderField
    label={field.label}
    bind:value={() => values[field.key], (v) => set(field.key, v)}
    min={field.min}
    max={field.max}
    step={field.step}
    unit={field.unit}
    factor={field.factor}
    width={56}
  />
  {#if field.key === "width"}
    <button
      type="button"
      class="icon-btn"
      class:on={linked}
      title={t("transform.field.link")}
      aria-pressed={linked}
      onmousedown={keepFocus}
      onclick={() => (linked = !linked)}
    >
      <Icon name={linked ? "link" : "linkBroken"} size={14} />
    </button>
  {/if}
{/each}

<style>
  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }
</style>
