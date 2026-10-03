<script lang="ts" module>
  import type { MessageKey } from "./i18n/en";
  import type { LengthUnit, ResolutionUnit } from "./units";

  /** What the new document's layer holds: a color, or nothing (a transparent layer). */
  export type NewBackground = "white" | "black" | "background" | "transparent";

  export type NewDocumentSettings = {
    /** `null`: untitled. */
    name: string | null;
    width: number;
    height: number;
    background: NewBackground;
    /** Pixels per inch (ADR 0028). */
    resolution: number;
  };

  /**
   * Common sizes, in pixels, and their resolution: print sizes at 300 ppi, screen sizes at 72
   * (Photoshop's).
   */
  const PRESETS: {
    id: string;
    label: MessageKey;
    width: number;
    height: number;
    ppi: number;
  }[] = [
    { id: "photo", label: "newDocument.preset.photo", width: 6000, height: 4000, ppi: 300 },
    { id: "a4", label: "newDocument.preset.a4", width: 2480, height: 3508, ppi: 300 },
    { id: "letter", label: "newDocument.preset.letter", width: 2550, height: 3300, ppi: 300 },
    { id: "hd", label: "newDocument.preset.hd", width: 1920, height: 1080, ppi: 72 },
    { id: "uhd", label: "newDocument.preset.uhd", width: 3840, height: 2160, ppi: 72 },
    { id: "square", label: "newDocument.preset.square", width: 1080, height: 1080, ppi: 72 },
    { id: "story", label: "newDocument.preset.story", width: 1080, height: 1920, ppi: 72 },
  ];

  const BACKGROUNDS: { value: NewBackground; label: MessageKey }[] = [
    { value: "white", label: "newDocument.background.white" },
    { value: "black", label: "newDocument.background.black" },
    { value: "background", label: "newDocument.background.color" },
    { value: "transparent", label: "newDocument.background.transparent" },
  ];

  /** Largest side, in pixels (as the engine accepts). */
  const MAX_SIDE = 300_000;

  /** As Photoshop's New dialog, the last settings come back (for the session). */
  let last = {
    width: 6000,
    height: 4000,
    background: "white" as NewBackground,
    resolution: 300,
    unit: "px" as LengthUnit,
    resolutionUnit: "ppi" as ResolutionUnit,
  };
</script>

<script lang="ts">
  // File > New, as Photoshop's New dialog: a name, a preset, the size in pixels, inches,
  // centimeters or millimeters with its orientation, the resolution (ADR 0028), and the
  // background contents; OK and Cancel on the right. Enter creates the document, Esc cancels.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import Icon from "./Icon.svelte";
  import {
    LENGTH_UNITS,
    MAX_PPI,
    MIN_PPI,
    RESOLUTION_UNITS,
    fromPixels,
    fromPpi,
    rounded,
    toPixels,
    toPpi,
  } from "./units";

  const LENGTH_LABELS: Record<LengthUnit, MessageKey> = {
    px: "sizeDialog.unit.px",
    in: "units.in",
    cm: "units.cm",
    mm: "units.mm",
  };
  const RESOLUTION_LABELS: Record<ResolutionUnit, MessageKey> = {
    ppi: "units.ppi",
    ppcm: "units.ppcm",
  };

  let {
    clipboard,
    oncreate,
    onclose,
  }: {
    /** The size of what the clipboard holds: a preset, chosen at first, as in Photoshop. */
    clipboard: [number, number] | null;
    oncreate: (settings: NewDocumentSettings) => void;
    onclose: () => void;
  } = $props();

  /** The presets, the clipboard's first. */
  const presets = $derived(
    clipboard
      ? [
          {
            id: "clipboard",
            label: "newDocument.preset.clipboard" as MessageKey,
            width: clipboard[0],
            height: clipboard[1],
            ppi: last.resolution,
          },
          ...PRESETS,
        ]
      : PRESETS,
  );

  let name = $state("");
  // svelte-ignore state_referenced_locally
  let width = $state(clipboard?.[0] ?? last.width);
  // svelte-ignore state_referenced_locally
  let height = $state(clipboard?.[1] ?? last.height);
  let background = $state(last.background);
  /** Pixels per inch. */
  let resolution = $state(last.resolution);
  let unit = $state<LengthUnit>(last.unit);
  let resolutionUnit = $state<ResolutionUnit>(last.resolutionUnit);
  /** The fields as typed: rewritten only when something else changes them. */
  let wField = $state("");
  let hField = $state("");
  let rField = $state("");

  function sync() {
    wField = String(rounded(fromPixels(width, unit, resolution), unit));
    hField = String(rounded(fromPixels(height, unit, resolution), unit));
    rField = String(rounded(fromPpi(resolution, resolutionUnit), resolutionUnit));
  }
  sync();

  /** A side typed in the unit: whole pixels. */
  function onSide(side: "width" | "height") {
    const value = Number(side === "width" ? wField : hField);
    if (!Number.isFinite(value)) return;
    const pixels = Math.round(toPixels(value, unit, resolution));
    if (side === "width") width = pixels;
    else height = pixels;
  }

  /** The resolution typed: a size in a length unit stays, its pixels follow (Photoshop). */
  function onResolution() {
    const next = toPpi(Number(rField), resolutionUnit);
    if (!Number.isFinite(next) || next < MIN_PPI || next > MAX_PPI) return;
    if (unit !== "px") {
      width = Math.round((width * next) / resolution);
      height = Math.round((height * next) / resolution);
    }
    resolution = next;
  }
  let dialog: HTMLDialogElement;
  let form: HTMLFormElement;

  const valid = $derived(
    [width, height].every((side) => Number.isInteger(side) && side >= 1 && side <= MAX_SIDE) &&
      resolution >= MIN_PPI &&
      resolution <= MAX_PPI,
  );
  /** The preset matching the size, either way round; none is Custom. */
  const preset = $derived(
    presets.find(
      (p) =>
        (p.width === width && p.height === height) || (p.width === height && p.height === width),
    )?.id ?? "custom",
  );
  const portrait = $derived(height > width);

  function choosePreset(id: string) {
    const chosen = presets.find((p) => p.id === id);
    if (!chosen) return;
    [width, height, resolution] = [chosen.width, chosen.height, chosen.ppi];
    sync();
  }

  /** Portrait or landscape: the sides swap if needed. */
  function orient(toPortrait: boolean) {
    if (toPortrait !== portrait && width !== height) {
      [width, height] = [height, width];
      sync();
    }
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!valid) return;
    last = { width, height, background, resolution, unit, resolutionUnit };
    const trimmed = name.trim();
    oncreate({ name: trimmed === "" ? null : trimmed, width, height, background, resolution });
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog. Enter on a closed list
    // creates the document, as in the fields (a select does not submit its form by itself).
    const keys = (e: KeyboardEvent) => {
      e.stopPropagation();
      if (e.key === "Enter" && e.target instanceof HTMLSelectElement) {
        e.preventDefault();
        form.requestSubmit();
      }
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="new-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="new-title">{t("newDocument.title")}</header>
  <form bind:this={form} onsubmit={submit}>
    <div class="fields">
      <label for="new-name">{t("newDocument.name")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="new-name"
        class="wide"
        type="text"
        placeholder={t("document.untitled")}
        bind:value={name}
        autofocus
      />
      <label for="new-preset">{t("newDocument.preset")}</label>
      <select
        id="new-preset"
        class="wide"
        value={preset}
        onchange={(e) => choosePreset((e.currentTarget as HTMLSelectElement).value)}
      >
        <option value="custom">{t("newDocument.preset.custom")}</option>
        {#each presets as p (p.id)}
          <option value={p.id}>{t(p.label)} ({p.width} × {p.height})</option>
        {/each}
      </select>
      <label for="new-width">{t("sizeDialog.width")}</label>
      <input
        id="new-width"
        type="number"
        min="0"
        step="any"
        bind:value={wField}
        oninput={() => onSide("width")}
      />
      <select
        aria-label={t("sizeDialog.unit")}
        value={unit}
        onchange={(e) => {
          unit = (e.currentTarget as HTMLSelectElement).value as LengthUnit;
          sync();
        }}
      >
        {#each LENGTH_UNITS as option (option)}
          <option value={option}>{t(LENGTH_LABELS[option])}</option>
        {/each}
      </select>
      <label for="new-height">{t("sizeDialog.height")}</label>
      <input
        id="new-height"
        type="number"
        min="0"
        step="any"
        bind:value={hField}
        oninput={() => onSide("height")}
      />
      <span class="unit">{t(LENGTH_LABELS[unit])}</span>
      <label for="new-resolution">{t("sizeDialog.resolution")}</label>
      <input
        id="new-resolution"
        type="number"
        min="0"
        step="any"
        bind:value={rField}
        oninput={onResolution}
      />
      <select
        aria-label={t("sizeDialog.resolution")}
        value={resolutionUnit}
        onchange={(e) => {
          resolutionUnit = (e.currentTarget as HTMLSelectElement).value as ResolutionUnit;
          sync();
        }}
      >
        {#each RESOLUTION_UNITS as option (option)}
          <option value={option}>{t(RESOLUTION_LABELS[option])}</option>
        {/each}
      </select>
      <span class="label">{t("newDocument.orientation")}</span>
      <div class="orientation" role="radiogroup" aria-label={t("newDocument.orientation")}>
        <button
          type="button"
          role="radio"
          aria-checked={portrait}
          class:selected={portrait}
          title={t("newDocument.portrait")}
          aria-label={t("newDocument.portrait")}
          onclick={() => orient(true)}
        >
          <Icon name="portrait" size={16} />
        </button>
        <button
          type="button"
          role="radio"
          aria-checked={!portrait}
          class:selected={!portrait}
          title={t("newDocument.landscape")}
          aria-label={t("newDocument.landscape")}
          onclick={() => orient(false)}
        >
          <Icon name="landscape" size={16} />
        </button>
      </div>
      <label for="new-background">{t("newDocument.background")}</label>
      <select id="new-background" class="wide" bind:value={background}>
        {#each BACKGROUNDS as entry (entry.value)}
          <option value={entry.value}>{t(entry.label)}</option>
        {/each}
      </select>
    </div>
    <div class="buttons">
      <button type="submit" class="btn primary" disabled={!valid}>{t("sizeDialog.ok")}</button>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 480px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  dialog::backdrop {
    background: #00000055;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  /* Photoshop's layout: the settings on the left, OK and Cancel stacked on the right. */
  form {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 14px;
    padding: 14px;
  }

  .fields {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    align-self: start;
    gap: 8px;
  }

  .fields > label,
  .label {
    grid-column: 1;
    color: var(--text-muted);
  }

  .wide {
    grid-column: 2 / -1;
  }

  input,
  select {
    min-width: 0;
  }

  .unit {
    color: var(--text-muted);
  }

  .orientation {
    display: flex;
    gap: 4px;
  }

  .orientation button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
    background: var(--chrome);
    color: var(--text-muted);
  }

  .orientation button.selected {
    border-color: var(--accent);
    color: var(--text);
  }

  .buttons {
    display: grid;
    align-content: start;
    gap: 6px;
  }

  .buttons .btn {
    min-width: 80px;
  }
</style>
