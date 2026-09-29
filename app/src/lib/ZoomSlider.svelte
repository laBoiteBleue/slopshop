<script lang="ts">
  import { MAX_ZOOM, MIN_ZOOM } from "./engine";
  import { formatZoom } from "./format";
  import { t } from "./i18n/index.svelte";

  let {
    zoom,
    hint,
    onzoom,
    onstep,
  }: {
    /** Current zoom (1 = 100%), or null before the first frame. */
    zoom: number | null;
    /** Tooltip of the percentage (navigation shortcuts). */
    hint: string;
    /** Zoom about the viewport center; settles once the view has taken it into account. */
    onzoom: (zoom: number) => Promise<void>;
    /** Step to the next zoom preset, like the keyboard shortcuts. */
    onstep: (zoomIn: boolean) => void;
  } = $props();

  // Logarithmic: equal distances are equal zoom ratios, over the engine's whole zoom range.
  const MIN = Math.log2(MIN_ZOOM);
  const MAX = Math.log2(MAX_ZOOM);

  // While the user moves the slider, it shows their value, not the view's: answers lag behind
  // the pointer, and writing them back into the slider would make it jump backwards. The draft
  // is dropped once the last request has been answered.
  let draft = $state<number | null>(null);
  let version = 0;
  let held = false;
  let lastRequest: Promise<void> = Promise.resolve();

  let position = $derived(draft ?? (zoom === null ? 0 : Math.log2(zoom)));
  let shown = $derived(draft === null ? zoom : 2 ** draft);

  function onInput(value: number) {
    if (!Number.isFinite(value)) return;
    draft = value;
    version++;
    lastRequest = onzoom(2 ** value);
  }

  function release() {
    const released = version;
    void lastRequest.then(() => {
      if (released === version && !held) draft = null;
    });
  }

  function onPointerDown() {
    held = true;
    const end = () => {
      window.removeEventListener("pointerup", end, true);
      window.removeEventListener("pointercancel", end, true);
      window.removeEventListener("blur", end);
      held = false;
      release();
    };
    window.addEventListener("pointerup", end, true);
    window.addEventListener("pointercancel", end, true);
    window.addEventListener("blur", end);
  }

  // Arrow and page keys step through the zoom presets instead of nudging the slider by an
  // imperceptible amount.
  function onKeyDown(e: KeyboardEvent) {
    const zoomIn = ["ArrowRight", "ArrowUp", "PageUp"].includes(e.key)
      ? true
      : ["ArrowLeft", "ArrowDown", "PageDown"].includes(e.key)
        ? false
        : null;
    if (zoomIn === null) return;
    e.preventDefault();
    onstep(zoomIn);
  }
</script>

<div class="zoom-control">
  <span class="readout" title={hint}>{shown === null ? "" : formatZoom(shown)}</span>
  <input
    type="range"
    min={MIN}
    max={MAX}
    step="any"
    value={position}
    disabled={zoom === null}
    aria-label={t("view.zoom")}
    aria-valuetext={shown === null ? undefined : formatZoom(shown)}
    title={t("view.zoom")}
    onpointerdown={onPointerDown}
    oninput={(e) => onInput(e.currentTarget.valueAsNumber)}
    onchange={release}
    onkeydown={onKeyDown}
  />
</div>

<style>
  .zoom-control {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* Fixed width (fits "6 400 %"): a readout growing while dragging would move the slider
     under the pointer. */
  .readout {
    flex: none;
    width: 7.5ch;
    text-align: right;
    color: var(--text);
    font-variant-numeric: tabular-nums;
    cursor: help;
  }

  input {
    width: 110px;
    height: 12px;
    margin: 0;
    accent-color: var(--accent);
  }
</style>
