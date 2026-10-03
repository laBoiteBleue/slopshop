<script lang="ts">
  // Free Transform's numbers in the options bar, as in Photoshop: X and Y of the reference
  // point, W and H in % (linked or not), the angle and the horizontal skew in degrees. A value
  // typed (Enter or leaving the field) transforms the box at once; Enter again applies.
  import type { Matrix } from "./engine";
  import Icon from "./Icon.svelte";
  import { getLocale, t } from "./i18n/index.svelte";
  import { keepFocus } from "./platform";
  import { compose, decompose, type TransformValues } from "./transformValues";

  let {
    matrix,
    pivot,
    onchange,
  }: {
    /** The transform so far (a map of the document's space). */
    matrix: Matrix;
    /** The reference point, in the coordinates of the box as it was. */
    pivot: [number, number];
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
    /** Shown value = stored × factor. */
    factor: number;
    digits: number;
  };
  const FIELDS = $derived<Field[]>([
    { key: "x", label: "X", unit: "px", factor: 1, digits: 1 },
    { key: "y", label: "Y", unit: "px", factor: 1, digits: 1 },
    { key: "width", label: t("transform.field.width"), unit: "%", factor: 100, digits: 1 },
    { key: "height", label: t("transform.field.height"), unit: "%", factor: 100, digits: 1 },
    { key: "angle", label: "∠", unit: "°", factor: DEGREES, digits: 1 },
    { key: "skew", label: t("transform.field.skew"), unit: "°", factor: DEGREES, digits: 1 },
  ]);

  function shown(field: Field): string {
    return new Intl.NumberFormat(getLocale(), {
      maximumFractionDigits: field.digits,
      useGrouping: false,
    }).format(values[field.key] * field.factor);
  }

  /** A value typed: in the interface's language (a decimal comma too). */
  function set(field: Field, text: string) {
    const typed = Number(text.trim().replace(",", "."));
    if (!Number.isFinite(typed)) return;
    const value = typed / field.factor;
    const next = { ...values, [field.key]: value };
    // Linked: the other side keeps its share of this one.
    if (linked && (field.key === "width" || field.key === "height")) {
      const other = field.key === "width" ? "height" : "width";
      const before = values[field.key];
      if (before !== 0) next[other] = values[other] * (value / before);
    }
    // A scale of zero cannot be undone.
    if (next.width === 0 || next.height === 0 || Math.abs(next.skew) >= Math.PI / 2) return;
    onchange(compose(next, pivot));
  }
</script>

{#each FIELDS as field (field.key)}
  <label class="field">
    <span class="name">{field.label}</span>
    <input
      type="text"
      inputmode="decimal"
      value={shown(field)}
      onchange={(e) => set(field, e.currentTarget.value)}
      onkeydown={(e) => {
        // Enter sets the value; the next Enter (outside the field) applies the transform.
        if (e.key === "Enter") {
          e.stopPropagation();
          set(field, e.currentTarget.value);
          e.currentTarget.blur();
        }
      }}
    />
    <span class="unit">{field.unit}</span>
  </label>
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
  .field {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  .name,
  .unit {
    color: var(--text-muted);
  }

  input {
    width: 52px;
  }

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }
</style>
