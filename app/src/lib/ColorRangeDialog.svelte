<script lang="ts" module>
  import type { ColorRangeRequest } from "./engine";
  import { eyedropperCursor, eyedropperFromKeys, type EyedropperKind } from "./eyedropper";

  /** Select > Color Range's samples and settings (document pixels). */
  export type ColorRangeState = {
    document: number;
    included: [number, number][];
    excluded: [number, number][];
    fuzziness: number;
    invert: boolean;
    /** What a click samples: a new color, one more, or one to take away. */
    eyedropper: EyedropperKind;
    /** Localized: each color selected only within `radius` document pixels of its sample. */
    localized: boolean;
    radius: number;
    /** The image as displayed, rather than the active layer (`layerId`, null without one). */
    sampleAll: boolean;
    layerId: number | null;
  };

  /** What the engine is asked for, from the dialog's state. */
  export function colorRangeRequest(range: ColorRangeState): ColorRangeRequest {
    return {
      included: range.included.map(([x, y]) => [x, y]),
      excluded: range.excluded.map(([x, y]) => [x, y]),
      fuzziness: range.fuzziness,
      invert: range.invert,
      localized: range.localized ? range.radius : null,
      layerId: range.sampleAll ? null : range.layerId,
    };
  }

  /** A sample at (x, y), as the eyedropper (or Shift / Alt) says. */
  export function sampleAt(
    range: ColorRangeState,
    x: number,
    y: number,
    keys: { shiftKey: boolean; altKey: boolean },
  ) {
    const kind = eyedropperFromKeys(range.eyedropper, keys);
    const point: [number, number] = [Math.floor(x), Math.floor(y)];
    if (kind === "add") range.included.push(point);
    else if (kind === "subtract") range.excluded.push(point);
    else {
      range.included = [point];
      range.excluded = [];
    }
  }
</script>

<script lang="ts">
  // Select > Color Range, as in Photoshop: click colors on the image or on the preview (Shift:
  // add one, Alt: take one away); Fuzziness widens them; Localized keeps each color near where
  // it was sampled; Invert. The panel stays beside the image (not modal) so that the image can
  // be clicked. The preview is the selection it would make, computed by the engine on the
  // document fitted in a small frame.
  import { onMount } from "svelte";
  import { engine } from "./engine";
  import SliderField from "./SliderField.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import type { MessageKey } from "./i18n/en";
  import { keepFocus } from "./platform";

  let {
    range = $bindable(),
    width,
    height,
    onapply,
    onclose,
  }: {
    range: ColorRangeState;
    /** Document size, pixels. */
    width: number;
    height: number;
    onapply: () => void;
    onclose: () => void;
  } = $props();

  const PREVIEW = 220;
  let canvas: HTMLCanvasElement;
  let request = 0;

  const EYEDROPPERS: { kind: ColorRangeState["eyedropper"]; icon: IconName; label: MessageKey }[] =
    [
      { kind: "pick", icon: "eyedropper", label: "colorRange.pick" },
      { kind: "add", icon: "eyedropperAdd", label: "colorRange.add" },
      { kind: "subtract", icon: "eyedropperSubtract", label: "colorRange.subtract" },
    ];

  // The preview follows the samples and settings: one request at a time, the latest wins.
  let busy = false;
  let again = false;
  async function refresh() {
    if (busy) {
      again = true;
      return;
    }
    busy = true;
    const id = ++request;
    try {
      const bytes = await engine.colorRangePreview(
        range.document,
        colorRangeRequest(range),
        PREVIEW,
      );
      if (id === request && canvas) {
        canvas.width = bytes.width;
        canvas.height = bytes.height;
        const rgba = new Uint8ClampedArray(bytes.width * bytes.height * 4);
        for (let i = 0; i < bytes.gray.length; i++) {
          rgba[i * 4] = rgba[i * 4 + 1] = rgba[i * 4 + 2] = bytes.gray[i];
          rgba[i * 4 + 3] = 255;
        }
        canvas.getContext("2d")?.putImageData(new ImageData(rgba, bytes.width, bytes.height), 0, 0);
      }
    } catch {
      // The preview only shows; the command itself reports errors.
    } finally {
      busy = false;
      if (again) {
        again = false;
        void refresh();
      }
    }
  }

  $effect(() => {
    // Dependencies: every sample and setting.
    void range.included.length;
    void range.excluded.length;
    void range.fuzziness;
    void range.invert;
    void range.localized;
    void range.radius;
    void range.sampleAll;
    void refresh();
  });

  /** Shift and Alt show on the pointer which eyedropper a click would use. */
  let keys = $state({ shiftKey: false, altKey: false });
  function track(e: PointerEvent | KeyboardEvent) {
    keys = { shiftKey: e.shiftKey, altKey: e.altKey };
  }

  function onPreviewClick(e: PointerEvent) {
    const box = canvas.getBoundingClientRect();
    const x = ((e.clientX - box.left) / box.width) * width;
    const y = ((e.clientY - box.top) / box.height) * height;
    if (x >= 0 && y >= 0 && x < width && y < height) sampleAt(range, x, y, e);
  }

  onMount(() => {
    // Enter applies (except in a number field, which takes its value) and Esc cancels, before
    // the app's own shortcuts see them.
    const keys = (e: KeyboardEvent) => {
      const field = e.target instanceof HTMLInputElement && e.target.type === "number";
      if (e.key === "Enter" && !field) onapply();
      else if (e.key === "Escape") onclose();
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });
</script>

<svelte:window onkeydown={track} onkeyup={track} />

<section class="panel" aria-labelledby="color-range-title">
  <header id="color-range-title" {@attach movable("color-range")}>{t("colorRange.title")}</header>
  <div class="body">
    <div class="eyedroppers" role="radiogroup" aria-label={t("colorRange.title")}>
      {#each EYEDROPPERS as entry (entry.kind)}
        <button
          class="icon-btn"
          class:on={range.eyedropper === entry.kind}
          role="radio"
          aria-checked={range.eyedropper === entry.kind}
          title={t(entry.label)}
          aria-label={t(entry.label)}
          onmousedown={keepFocus}
          onclick={() => (range.eyedropper = entry.kind)}
        >
          <Icon name={entry.icon} />
        </button>
      {/each}
    </div>
    <label class="row">
      <span>{t("colorRange.fuzziness")}</span>
      <input type="range" min="0" max="200" step="1" bind:value={range.fuzziness} />
      <input type="number" min="0" max="200" step="1" bind:value={range.fuzziness} />
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={range.localized} />
      {t("colorRange.localized")}
    </label>
    {#if range.localized}
      <SliderField
        label={t("colorRange.radius")}
        bind:value={range.radius}
        min={1}
        max={Math.max(width, height)}
        unit="px"
        log
      />
    {/if}
    <label class="check">
      <input type="checkbox" bind:checked={range.sampleAll} disabled={range.layerId === null} />
      {t("colorRange.sampleAll")}
    </label>
    <canvas
      bind:this={canvas}
      class="preview"
      style:aspect-ratio="{width} / {height}"
      style:cursor={eyedropperCursor(eyedropperFromKeys(range.eyedropper, keys))}
      onpointermove={track}
      onpointerdown={onPreviewClick}
    ></canvas>
    <p class="hint">{t("colorRange.hint")}</p>
    <label class="check">
      <input type="checkbox" bind:checked={range.invert} />
      {t("colorRange.invert")}
    </label>
  </div>
  <footer>
    <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
    <button
      type="button"
      class="btn primary"
      disabled={range.included.length === 0 && !range.invert}
      onclick={onapply}
    >
      {t("sizeDialog.ok")}
    </button>
  </footer>
</section>

<style>
  .panel {
    position: fixed;
    top: 96px;
    right: 276px;
    z-index: 15;
    display: grid;
    width: 250px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    box-shadow: 0 10px 32px #0009;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .body {
    display: grid;
    gap: 8px;
    padding: 10px;
  }

  .eyedroppers {
    display: flex;
    gap: 4px;
  }

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }

  .row {
    display: grid;
    grid-template-columns: auto 1fr 48px;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }

  .preview {
    width: 100%;
    background: #000000;
    image-rendering: auto;
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .check input {
    margin: 0;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
