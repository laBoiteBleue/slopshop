<script lang="ts">
  // A text layer's Character settings in the Properties panel (ADR 0041, Photoshop's Character
  // and Paragraph panels in one section): font family and style, size and leading in points,
  // tracking, color, Photoshop's row of style buttons (Faux Bold to Strikethrough) and Standard
  // Ligatures, and the paragraph's alignment. While the text is typed they show and set the
  // characters selected; at a caret, the character before it with the style chosen there for
  // the next characters typed (Photoshop's pending style); not typed, the whole text. A
  // setting the characters do not share shows empty (`textStyle.ts`).
  import { untrack } from "svelte";
  import type { CharStyleDto, FontsView, TextAlign, TextDto } from "./engine";
  import { styleName } from "./fontStyleName";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { keepFocus } from "./platform";
  import { srgbToHex } from "./color";
  import { isPressed, STYLE_BUTTONS, styleAt, toggleStyle, type CharToggle } from "./textStyle";
  import {
    clampSizePt,
    MAX_SIZE_PT,
    MIN_SIZE_PT,
    pixelsToPoints,
    pointsToPixels,
    styleKey,
    styleOfKey,
  } from "./typeText";

  let {
    text,
    range,
    pending = null,
    ppi,
    fonts,
    stylesOf,
    onstyle,
    onalign,
    onpickcolor,
  }: {
    text: TextDto;
    /**
     * The characters selected while the text is typed, or its caret (UTF-16 units); null: the
     * whole text (not typed).
     */
    range: [number, number] | null;
    /** The style chosen at the caret for the next characters typed. */
    pending?: CharStyleDto | null;
    /** The document's resolution, through which points are pixels (ADR 0028). */
    ppi: number;
    fonts: FontsView | null;
    /** The styles a family offers. */
    stylesOf: (family: string) => Promise<{ weight: number; italic: boolean }[]>;
    onstyle: (change: CharStyleDto) => void;
    onalign: (align: TextAlign) => void;
    /** The color swatch was clicked: the app lets a color be chosen. */
    onpickcolor: () => void;
  } = $props();

  const shown = $derived(
    range ? styleAt(text, range[0], range[1], pending) : styleAt(text, 0, text.content.length),
  );
  const settings = $derived(shown.settings);
  const mixed = $derived(new Set(shown.mixed));

  /** The family shown (a value: typing does not ask its styles again). */
  const family = $derived(settings.family);
  /** The styles of the family shown. */
  let styles = $state<{ weight: number; italic: boolean }[]>([]);
  $effect(() => {
    const asked = family;
    void untrack(() => stylesOf(asked))
      .then((found) => {
        if (family === asked) styles = found;
      })
      .catch(() => (styles = []));
  });

  /** Points shown to the hundredth, as Photoshop shows them. */
  const points = (px: number) => Math.round(pixelsToPoints(px, ppi) * 100) / 100;

  /** A number typed in a field, or null (the field shows the text's again). */
  function typed(e: Event): number | null {
    const input = e.currentTarget as HTMLInputElement;
    const value = input.valueAsNumber;
    if (Number.isFinite(value)) return value;
    input.value = input.defaultValue;
    return null;
  }

  function setSize(e: Event) {
    const pt = typed(e);
    if (pt !== null) onstyle({ size: pointsToPixels(clampSizePt(pt, ppi), ppi) });
  }

  function setLeading(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    // Emptied: automatic leading.
    if (input.value.trim() === "") {
      onstyle({ leading: null });
      return;
    }
    const pt = typed(e);
    if (pt !== null) onstyle({ leading: pointsToPixels(Math.max(pt, 0), ppi) });
  }

  function setTracking(e: Event) {
    const value = typed(e);
    if (value !== null) onstyle({ tracking: Math.min(Math.max(Math.round(value), -1000), 1000) });
  }

  /** A style button: pressed, or not, by the characters' settings. */
  const pressed = (toggle: CharToggle) => isPressed(toggle, settings, shown.mixed);

  const ALIGNS = [
    { align: "left", icon: "textLeft" },
    { align: "center", icon: "textCenter" },
    { align: "right", icon: "textRight" },
  ] as const;
</script>

<section class="panel" aria-label={t("properties.character.title")}>
  <div class="title">{t("properties.character.title")}</div>
  <div class="row">
    <select
      class="wide"
      aria-label={t("options.type.family")}
      title={t("options.type.family")}
      value={mixed.has("family") ? "" : settings.family}
      onchange={(e) => onstyle({ family: e.currentTarget.value })}
    >
      {#if mixed.has("family")}
        <option value="" disabled></option>
      {:else if !fonts?.families.includes(settings.family)}
        <option value={settings.family}>{settings.family}</option>
      {/if}
      {#each fonts?.families ?? [] as name (name)}
        <option value={name}>{name}</option>
      {/each}
    </select>
  </div>
  <div class="row">
    {#if mixed.has("weight") || mixed.has("italic")}
      <select
        class="wide"
        aria-label={t("options.type.style")}
        title={t("options.type.style")}
        value=""
        onchange={(e) => onstyle(styleOfKey(e.currentTarget.value))}
      >
        <option value="" disabled></option>
        {#each styles as style (styleKey(style))}
          <option value={styleKey(style)}>{styleName(style)}</option>
        {/each}
      </select>
    {:else}
      <select
        class="wide"
        aria-label={t("options.type.style")}
        title={t("options.type.style")}
        value={styleKey(settings)}
        onchange={(e) => onstyle(styleOfKey(e.currentTarget.value))}
      >
        {#if !styles.some((s) => styleKey(s) === styleKey(settings))}
          <option value={styleKey(settings)}>{styleName(settings)}</option>
        {/if}
        {#each styles as style (styleKey(style))}
          <option value={styleKey(style)}>{styleName(style)}</option>
        {/each}
      </select>
    {/if}
  </div>
  <div class="row">
    <label for="character-size">{t("options.type.size")}</label>
    <input
      id="character-size"
      type="number"
      min={MIN_SIZE_PT}
      max={MAX_SIZE_PT}
      step="0.1"
      value={mixed.has("size") ? "" : points(settings.size)}
      onchange={setSize}
    />
    <span class="unit">pt</span>
  </div>
  <div class="row">
    <label for="character-leading">{t("properties.character.leading")}</label>
    <input
      id="character-leading"
      type="number"
      min="0"
      step="0.1"
      placeholder={mixed.has("leading") ? "" : t("properties.character.auto")}
      value={mixed.has("leading") || settings.leading === null ? "" : points(settings.leading)}
      onchange={setLeading}
    />
    <span class="unit">pt</span>
  </div>
  <div class="row">
    <label for="character-tracking">{t("properties.character.tracking")}</label>
    <input
      id="character-tracking"
      type="number"
      min="-1000"
      max="1000"
      step="10"
      value={mixed.has("tracking") ? "" : settings.tracking}
      onchange={setTracking}
    />
  </div>
  <div class="row">
    <span class="label">{t("options.type.color")}</span>
    <button
      type="button"
      class="swatch"
      class:mixed={mixed.has("color")}
      style:background={mixed.has("color") ? null : srgbToHex(settings.color)}
      title={t("options.type.color")}
      aria-label={t("options.type.color")}
      onmousedown={keepFocus}
      onclick={onpickcolor}
    ></button>
  </div>
  <div class="row buttons">
    {#each STYLE_BUTTONS as toggle (toggle)}
      <button
        type="button"
        class="icon-btn"
        class:on={pressed(toggle)}
        aria-pressed={pressed(toggle)}
        title={t(`properties.character.${toggle}`)}
        aria-label={t(`properties.character.${toggle}`)}
        onmousedown={keepFocus}
        onclick={() => onstyle(toggleStyle(toggle, pressed(toggle)))}
      >
        <Icon name={toggle} />
      </button>
    {/each}
  </div>
  <div class="row buttons">
    <button
      type="button"
      class="icon-btn"
      class:on={pressed("ligatures")}
      aria-pressed={pressed("ligatures")}
      title={t("properties.character.ligatures")}
      aria-label={t("properties.character.ligatures")}
      onmousedown={keepFocus}
      onclick={() => onstyle(toggleStyle("ligatures", pressed("ligatures")))}
    >
      <Icon name="ligatures" />
    </button>
  </div>
  <div class="row">
    {#each ALIGNS as entry (entry.align)}
      <button
        type="button"
        class="icon-btn"
        class:on={text.align === entry.align}
        aria-pressed={text.align === entry.align}
        title={t(`options.type.align.${entry.align}`)}
        aria-label={t(`options.type.align.${entry.align}`)}
        onmousedown={keepFocus}
        onclick={() => onalign(entry.align)}
      >
        <Icon name={entry.icon} />
      </button>
    {/each}
  </div>
</section>

<style>
  /* As the Properties panel's other sections. */
  .panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    background: var(--panel);
  }

  .title {
    padding: 6px 8px 2px;
    font-weight: 600;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
  }

  .row > label,
  .row > .label {
    min-width: 64px;
  }

  .row input[type="number"] {
    width: 64px;
  }

  /* Photoshop's row of buttons, side by side. */
  .row.buttons {
    gap: 2px;
    flex-wrap: wrap;
  }

  .wide {
    flex: 1;
    min-width: 0;
  }

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }

  .unit {
    color: var(--text-muted);
  }

  .swatch {
    width: 40px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: 2px;
  }

  /* Characters of several colors: a neutral checker. */
  .swatch.mixed {
    background: repeating-conic-gradient(var(--fill) 0 25%, var(--panel) 0 50%) 50% / 8px 8px;
  }
</style>
