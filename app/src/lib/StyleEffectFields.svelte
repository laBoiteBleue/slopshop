<script lang="ts">
  // The settings of a shadow (Drop Shadow, Inner Shadow) or a glow (Outer Glow, Inner Glow) in
  // the Layer Style dialog (ADR 0032): blend mode, color, opacity, then for a shadow its angle
  // and distance, then Spread (Choke inside) and Size; every number with a slider. They edit the
  // effect in place.
  import { BLEND_MODE_GROUPS, type StyleGlow, type StyleShadow } from "./engine";
  import { srgbToHex } from "./color";
  import SliderField from "./SliderField.svelte";
  import { t } from "./i18n/index.svelte";

  let {
    effect = $bindable(),
    inside,
    onpickcolor,
  }: {
    effect: StyleShadow | StyleGlow;
    /** An inner effect: Spread reads Choke. */
    inside: boolean;
    onpickcolor: () => void;
  } = $props();
</script>

<label class="field">
  <span>{t("style.mode")}</span>
  <select bind:value={effect.mode}>
    {#each BLEND_MODE_GROUPS as group, i (i)}
      {#if i > 0}<hr />{/if}
      {#each group as mode (mode)}
        <option value={mode}>{t(`blendMode.${mode}`)}</option>
      {/each}
    {/each}
  </select>
</label>
<div class="field">
  <span>{t("style.color")}</span>
  <button
    type="button"
    class="swatch"
    style:background={srgbToHex(effect.color)}
    title={t("style.color")}
    aria-label={t("style.color")}
    onclick={onpickcolor}
  ></button>
</div>
<SliderField
  label={t("style.opacity")}
  bind:value={effect.opacity}
  min={0}
  max={100}
  unit="%"
  factor={100}
/>
{#if "angle" in effect}
  <SliderField label={t("style.angle")} bind:value={effect.angle} min={-180} max={180} unit="°" />
  <SliderField
    label={t("style.distance")}
    bind:value={effect.distance}
    min={0}
    max={30000}
    unit="px"
  />
{/if}
<SliderField
  label={t(inside ? "style.choke" : "style.spread")}
  bind:value={effect.spread}
  min={0}
  max={100}
  unit="%"
/>
<SliderField label={t("style.size")} bind:value={effect.size} min={0} max={250} unit="px" />

<style>
  .field {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .field > span {
    min-width: 90px;
  }

  .swatch {
    width: 40px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 2px;
    cursor: pointer;
  }
</style>
