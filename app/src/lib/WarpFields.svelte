<script lang="ts">
  // Warp's settings in the options bar, as in Photoshop: the split modes (a click on the grid
  // then splits it there), the grid's size (Default, 4 × 4, 5 × 5), the style (None, Custom,
  // and Photoshop's fifteen styles from Arc to Twist), its orientation and its bend (-100 to 100 %). Each change
  // warps the layer live (ADR 0044).
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { keepFocus } from "./platform";
  import SliderField from "./SliderField.svelte";
  import { clampBend, DEFAULT_BEND, isStyle, WARP_STYLES, type WarpSettings } from "./warp";
  import { GRID_SIZES, type SplitMode } from "./warpGrid";

  let {
    settings,
    size = 3,
    split = null,
    splittable = true,
    onchange,
    onsize = () => {},
    onsplit = () => {},
  }: {
    settings: WarpSettings;
    /** The grid's patches per side when it is one of GRID_SIZES; null for any other grid. */
    size?: number | null;
    /** The Split Warp mode on, if any. */
    split?: SplitMode | null;
    /** Whether the grid can take another split (fewer than the most patches). */
    splittable?: boolean;
    onchange: (settings: WarpSettings) => void;
    /** A grid size chosen: patches per side. */
    onsize?: (size: number) => void;
    /** A Split Warp mode turned on, or off (null). */
    onsplit?: (mode: SplitMode | null) => void;
  } = $props();

  const SPLITS = [
    { mode: "crosswise", icon: "splitCrosswise" },
    { mode: "vertical", icon: "splitVertical" },
    { mode: "horizontal", icon: "splitHorizontal" },
  ] as const;

  function chooseStyle(value: string) {
    const style: WarpSettings["style"] =
      value === "custom" ? "custom" : (WARP_STYLES.find((s) => s === value) ?? null);
    // A style chosen from None or Custom starts at Photoshop's bend.
    const bend = isStyle(settings) || !style || style === "custom" ? settings.bend : DEFAULT_BEND;
    onchange({ ...settings, style, bend });
  }
</script>

<span class="fields">
  {#each SPLITS as entry (entry.mode)}
    <button
      class="icon-btn"
      class:on={split === entry.mode}
      aria-pressed={split === entry.mode}
      disabled={!splittable}
      onmousedown={keepFocus}
      title={t(`warp.split.${entry.mode}`)}
      aria-label={t(`warp.split.${entry.mode}`)}
      onclick={() => onsplit(split === entry.mode ? null : entry.mode)}
    >
      <Icon name={entry.icon} />
    </button>
  {/each}
  <label class="field">
    <span>{t("warp.grid")}</span>
    <select
      aria-label={t("warp.grid")}
      value={size === null ? "custom" : String(size)}
      onchange={(e) => {
        const n = Number(e.currentTarget.value);
        if (Number.isInteger(n)) onsize(n);
      }}
    >
      {#each GRID_SIZES as n (n)}
        <option value={String(n)}>{n === 3 ? t("warp.grid.default") : `${n} × ${n}`}</option>
      {/each}
      {#if size === null}
        <option value="custom" disabled>{t("warp.style.custom")}</option>
      {/if}
    </select>
  </label>
  <label class="field">
    <span>{t("warp.style")}</span>
    <select
      aria-label={t("warp.style")}
      value={settings.style ?? "none"}
      onchange={(e) => chooseStyle(e.currentTarget.value)}
    >
      <option value="none">{t("warp.style.none")}</option>
      <option value="custom">{t("warp.style.custom")}</option>
      {#each WARP_STYLES as style (style)}
        <option value={style}>{t(`warp.style.${style}`)}</option>
      {/each}
    </select>
  </label>
  <select
    aria-label={t("warp.orientation")}
    value={settings.vertical ? "vertical" : "horizontal"}
    disabled={!isStyle(settings)}
    onchange={(e) => onchange({ ...settings, vertical: e.currentTarget.value === "vertical" })}
  >
    <option value="horizontal">{t("warp.horizontal")}</option>
    <option value="vertical">{t("warp.vertical")}</option>
  </select>
  <span class="bend" class:disabled={!isStyle(settings)} inert={!isStyle(settings)}>
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

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }

  .bend {
    display: contents;
  }

  .bend.disabled > :global(*) {
    opacity: 0.45;
  }
</style>
