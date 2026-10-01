<script lang="ts" module>
  /** Select > Color Range's samples and settings (document pixels). */
  export type ColorRangeState = {
    document: number;
    included: [number, number][];
    excluded: [number, number][];
    fuzziness: number;
    invert: boolean;
    /** What a click samples: a new color, one more, or one to take away. */
    eyedropper: "pick" | "add" | "subtract";
  };

  /** A sample at (x, y), as the eyedropper (or Shift / Alt) says. */
  export function sampleAt(
    range: ColorRangeState,
    x: number,
    y: number,
    keys: { shiftKey: boolean; altKey: boolean },
  ) {
    const kind = keys.shiftKey ? "add" : keys.altKey ? "subtract" : range.eyedropper;
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
  // add one, Alt: take one away); Fuzziness widens them; Invert. The panel stays beside the
  // image (not modal) so that the image can be clicked. The preview is the selection it would
  // make, computed by the engine on the document fitted in a small frame.
  import { onMount } from "svelte";
  import { engine } from "./engine";
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
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
        {
          included: $state.snapshot(range.included),
          excluded: $state.snapshot(range.excluded),
          fuzziness: range.fuzziness,
          invert: range.invert,
          layerId: null,
        },
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
    void refresh();
  });

  function onPreviewClick(e: PointerEvent) {
    const box = canvas.getBoundingClientRect();
    const x = ((e.clientX - box.left) / box.width) * width;
    const y = ((e.clientY - box.top) / box.height) * height;
    if (x >= 0 && y >= 0 && x < width && y < height) sampleAt(range, x, y, e);
  }

  onMount(() => {
    // Enter applies and Esc cancels, before the app's own shortcuts see them.
    const keys = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement && e.target.type === "number") return;
      if (e.key === "Enter") onapply();
      else if (e.key === "Escape") onclose();
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });
</script>

<section class="panel" aria-labelledby="color-range-title">
  <header id="color-range-title">{t("colorRange.title")}</header>
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
    <canvas
      bind:this={canvas}
      class="preview"
      style:aspect-ratio="{width} / {height}"
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
    cursor: crosshair;
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
