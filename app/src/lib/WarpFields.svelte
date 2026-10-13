<script lang="ts">
  // Warp's settings in the options bar, as in Photoshop: the style (None, Arc, Bulge, Flag,
  // Wave, Twist), its orientation and its bend (-100 to 100 %). Each change warps the layer
  // live (ADR 0044).
  import { t } from "./i18n/index.svelte";
  import SliderField from "./SliderField.svelte";
  import { clampBend, DEFAULT_BEND, WARP_STYLES, type WarpSettings } from "./warp";
  import type { WarpStyle } from "./engine";

  let {
    settings,
    onchange,
  }: {
    settings: WarpSettings;
    onchange: (settings: WarpSettings) => void;
  } = $props();

  function chooseStyle(value: string) {
    const style = WARP_STYLES.find((s) => s === value) ?? null;
    // A style chosen from None starts at Photoshop's bend.
    const bend = settings.style || !style ? settings.bend : DEFAULT_BEND;
    onchange({ ...settings, style: style as WarpStyle | null, bend });
  }
</script>

<span class="fields">
  <label class="field">
    <span>{t("warp.style")}</span>
    <select
      aria-label={t("warp.style")}
      value={settings.style ?? "none"}
      onchange={(e) => chooseStyle(e.currentTarget.value)}
    >
      <option value="none">{t("warp.style.none")}</option>
      {#each WARP_STYLES as style (style)}
        <option value={style}>{t(`warp.style.${style}`)}</option>
      {/each}
    </select>
  </label>
  <select
    aria-label={t("warp.orientation")}
    value={settings.vertical ? "vertical" : "horizontal"}
    disabled={!settings.style}
    onchange={(e) => onchange({ ...settings, vertical: e.currentTarget.value === "vertical" })}
  >
    <option value="horizontal">{t("warp.horizontal")}</option>
    <option value="vertical">{t("warp.vertical")}</option>
  </select>
  <span class="bend" class:disabled={!settings.style} inert={!settings.style}>
    <SliderField
      label={t("warp.bend")}
      bind:value={() => settings.bend, (v) => onchange({ ...settings, bend: clampBend(v) })}
      min={-100}
      max={100}
      step={1}
      unit="%"
      width={48}
    />
  </span>
</span>

<style>
  .fields {
    display: contents;
  }

  .field {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .bend {
    display: contents;
  }

  .bend.disabled > :global(*) {
    opacity: 0.45;
  }
</style>
