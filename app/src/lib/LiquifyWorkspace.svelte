<script lang="ts">
  // Filter > Liquify's workspace (ADR 0037): a large modal window, the layer previewed live
  // through its displacement field, the tools on the left (keys W R E C S B O F D; Alt turns the
  // twirl, bloats a pucker, thaws a freeze), the brush's settings on the right, OK and Cancel.
  // Navigated as the rest of the app: the middle button (or Space) pans, the wheel zooms about the
  // pointer. The engine edits the field; this only sends strokes and draws the frames it
  // returns. OK makes the field one entry of the layer's stack; Cancel leaves everything as it was.
  import { onMount, untrack } from "svelte";
  import { engine, type LiquifyBrush, type LiquifyState, type LiquifyToolId } from "./engine";
  import { t } from "./i18n/index.svelte";
  import Icon from "./Icon.svelte";
  import {
    DEFAULT_BRUSH,
    LIQUIFY_TOOLS,
    BRUSH_LIMITS,
    effectiveTool,
    fitView,
    frameRequest,
    panned,
    steppedSize,
    toLayer,
    toolForKey,
    toolbarTool,
    wheelZoom,
    zoomAt,
    zoomPercent,
    type View,
  } from "./liquify";
  import { FrameLoop, StrokeQueue } from "./liquifyLoop";
  import SliderField from "./SliderField.svelte";
  import { wheelPixels } from "./viewMapping";

  let {
    documentId,
    layerId,
    index,
    onok,
    oncancel,
  }: {
    documentId: number;
    layerId: number;
    /** The Liquify entry of the layer's stack edited again, or null for a new one. */
    index: number | null;
    /** OK: the workspace's field is in the engine, ready to become an entry. */
    onok: () => void;
    oncancel: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let stage: HTMLDivElement;
  let canvas: HTMLCanvasElement;

  /** The session, once the engine opened it (null while it does, or if it could not). */
  let session = $state<LiquifyState | null>(null);
  let failed = $state(false);
  let tool = $state<LiquifyToolId>("forwardWarp");
  let alt = $state(false);
  let space = $state(false);
  let brush = $state<LiquifyBrush>({ ...DEFAULT_BRUSH });
  let showFrozen = $state(true);
  /** The stage's size, CSS pixels. */
  let box = $state({ width: 0, height: 0 });
  let view = $state<View>({ x: 0, y: 0, zoom: 1 });
  /** The view was changed by the user: the stage's resizing no longer fits the layer again. */
  let navigated = false;
  /** Where the pointer is over the stage (CSS pixels), for the brush's outline. */
  let pointer = $state<{ x: number; y: number } | null>(null);

  const active = $derived(effectiveTool(tool, alt));
  const dpr = () => (typeof window === "undefined" ? 1 : window.devicePixelRatio || 1);

  // --- Strokes and frames ------------------------------------------------------------------

  const queue = new StrokeQueue(
    (piece) => engine.liquifyStroke(documentId, piece).then((state) => (session = state)),
    () => loop.invalidate(),
  );

  const loop: FrameLoop<{
    pixels: Uint8ClampedArray<ArrayBuffer>;
    width: number;
    height: number;
  }> = new FrameLoop({
    settle: () => queue.idle(),
    frame: async () => {
      const request = frameRequest(view, box, dpr(), showFrozen);
      const pixels = await engine.liquifyFrame(documentId, request);
      return { pixels, width: request.width, height: request.height };
    },
    draw: ({ pixels, width, height }) => {
      if (canvas.width !== width) canvas.width = width;
      if (canvas.height !== height) canvas.height = height;
      canvas.getContext("2d")?.putImageData(new ImageData(pixels, width, height), 0, 0);
    },
    next: () => new Promise((resolve) => requestAnimationFrame(resolve)),
  });

  // A frame is wanted when the view, the stage or the overlay changes.
  $effect(() => {
    void [view.x, view.y, view.zoom, box.width, box.height, showFrozen, session !== null];
    if (untrack(() => session !== null && box.width > 0)) loop.invalidate();
  });

  // The layer fits the stage until the user moves the view.
  $effect(() => {
    if (!session || box.width <= 0 || navigated) return;
    view = fitView(session, box);
  });

  onMount(() => {
    dialog.showModal();
    // The keys (Enter, the tools) act on the stage, not on a button the browser focused.
    stage.focus();
    const observer = new ResizeObserver(() => {
      const rect = stage.getBoundingClientRect();
      box = { width: rect.width || stage.clientWidth, height: rect.height || stage.clientHeight };
    });
    observer.observe(stage);
    stage.addEventListener("wheel", onwheel, { passive: false });
    window.addEventListener("keydown", onkeydown, true);
    window.addEventListener("keyup", onkeyup, true);
    window.addEventListener("blur", releaseKeys);
    engine
      .liquifyOpen(documentId, layerId, index)
      .then((opened) => (session = opened))
      .catch(() => (failed = true));
    return () => {
      loop.stop();
      stopHold();
      observer.disconnect();
      stage.removeEventListener("wheel", onwheel);
      window.removeEventListener("keydown", onkeydown, true);
      window.removeEventListener("keyup", onkeyup, true);
      window.removeEventListener("blur", releaseKeys);
    };
  });

  // --- The pointer ---------------------------------------------------------------------------

  /** A stroke is under way with this pointer, or a pan. */
  let drawing: number | null = null;
  let panning = $state<{ pointerId: number; x: number; y: number } | null>(null);
  /** When the pointer last moved, to tell holding still from moving. */
  let movedAt = 0;
  let holdTimer: ReturnType<typeof setInterval> | null = null;
  let heldAt = 0;

  function point(e: { clientX: number; clientY: number }): [number, number] {
    const rect = stage.getBoundingClientRect();
    return [e.clientX - rect.left, e.clientY - rect.top];
  }

  function layerPoint(e: { clientX: number; clientY: number }): [number, number] {
    const [x, y] = point(e);
    return toLayer(view, x, y);
  }

  function onpointerdown(e: PointerEvent) {
    if (!session || failed) return;
    const pan = e.button === 1 || (e.button === 0 && space);
    if (pan) {
      e.preventDefault();
      stage.setPointerCapture(e.pointerId);
      panning = { pointerId: e.pointerId, x: e.clientX, y: e.clientY };
      return;
    }
    if (e.button !== 0 || drawing !== null) return;
    stage.setPointerCapture(e.pointerId);
    drawing = e.pointerId;
    movedAt = performance.now();
    queue.begin(active, { ...brush }, layerPoint(e));
    startHold();
  }

  function onpointermove(e: PointerEvent) {
    pointer = { x: point(e)[0], y: point(e)[1] };
    if (panning && e.pointerId === panning.pointerId) {
      navigated = true;
      view = panned(view, e.clientX - panning.x, e.clientY - panning.y);
      panning = { ...panning, x: e.clientX, y: e.clientY };
      return;
    }
    if (drawing === e.pointerId) {
      movedAt = performance.now();
      queue.move(layerPoint(e));
    }
  }

  function onpointerup(e: PointerEvent) {
    if (panning && e.pointerId === panning.pointerId) panning = null;
    if (drawing === e.pointerId) {
      drawing = null;
      stopHold();
      queue.end();
    }
  }

  function onwheel(e: WheelEvent) {
    e.preventDefault();
    const dy = wheelPixels(e.deltaY, e.deltaMode, stage.clientHeight);
    if (dy === 0) return;
    const [x, y] = point(e);
    navigated = true;
    view = zoomAt(view, wheelZoom(dy), x, y);
  }

  /** The tools that act while held still do, on a timer, while the pointer stays where it is. */
  function startHold() {
    stopHold();
    heldAt = performance.now();
    holdTimer = setInterval(() => {
      const now = performance.now();
      const seconds = (now - heldAt) / 1000;
      heldAt = now;
      // Only while the pointer is still: a moving pointer does its work by moving.
      if (now - movedAt >= 60) queue.hold(seconds);
    }, 33);
  }

  function stopHold() {
    if (holdTimer !== null) clearInterval(holdTimer);
    holdTimer = null;
  }

  // --- The keyboard --------------------------------------------------------------------------

  function typing(e: KeyboardEvent): boolean {
    const target = e.target as HTMLElement | null;
    return (
      !!target &&
      (target.tagName === "INPUT" || target.tagName === "SELECT") &&
      target.getAttribute("type") !== "range" &&
      target.getAttribute("type") !== "checkbox"
    );
  }

  function releaseKeys() {
    alt = false;
    space = false;
  }

  function onkeydown(e: KeyboardEvent) {
    // Modal: the app's shortcuts must not act behind the workspace.
    e.stopPropagation();
    if (e.key === "Alt") {
      e.preventDefault();
      alt = true;
      return;
    }
    if (typing(e)) return;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && !e.altKey) {
      const key = e.key.toLowerCase();
      if (key === "z") {
        e.preventDefault();
        void undo(e.shiftKey);
      } else if (key === "y") {
        e.preventDefault();
        void undo(true);
      }
      return;
    }
    if (e.key === " ") {
      e.preventDefault();
      space = true;
      return;
    }
    if (e.key === "[" || e.key === "]") {
      e.preventDefault();
      brush = { ...brush, size: steppedSize(brush.size, e.key === "]" ? 1 : -1) };
      return;
    }
    // Enter accepts, as in Photoshop, unless it presses a button other than a tool.
    const pressed = e.target instanceof Element ? e.target.closest("button:not(.tool)") : null;
    if (e.key === "Enter" && !pressed) {
      e.preventDefault();
      void ok();
      return;
    }
    if (!mod && !e.altKey) {
      const chosen = toolForKey(e.key);
      if (chosen) {
        e.preventDefault();
        tool = chosen;
      }
    }
  }

  function onkeyup(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Alt") alt = false;
    if (e.key === " ") space = false;
  }

  // --- Commands ------------------------------------------------------------------------------

  async function undo(redo: boolean) {
    if (!session || drawing !== null) return;
    await queue.idle();
    session = await engine.liquifyUndo(documentId, redo);
    loop.invalidate();
  }

  async function restoreAll() {
    if (!session || drawing !== null) return;
    await queue.idle();
    session = await engine.liquifyRestoreAll(documentId);
    loop.invalidate();
  }

  async function ok() {
    if (!session || failed || drawing !== null) return;
    await queue.idle();
    onok();
  }

  function cancel() {
    oncancel();
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="liquify-title"
  oncancel={(e) => {
    e.preventDefault();
    cancel();
  }}
>
  <header id="liquify-title">{t("liquify.title")}</header>
  <div class="body">
    <div class="tools" role="toolbar" aria-label={t("liquify.tools")} aria-orientation="vertical">
      {#each LIQUIFY_TOOLS as entry (entry.id)}
        <button
          type="button"
          class="tool"
          class:on={toolbarTool(active) === entry.id}
          title={t("liquify.toolTitle", { name: t(entry.label), key: entry.key.toUpperCase() })}
          aria-label={t(entry.label)}
          aria-pressed={toolbarTool(active) === entry.id}
          onclick={() => (tool = entry.id)}
        >
          <Icon name={entry.icon} size={20} />
        </button>
      {/each}
    </div>

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="stage"
      tabindex="-1"
      class:panning={space || panning !== null}
      bind:this={stage}
      role="application"
      aria-label={t("liquify.canvas")}
      {onpointerdown}
      {onpointermove}
      {onpointerup}
      onpointercancel={onpointerup}
      onpointerleave={() => (pointer = null)}
    >
      <canvas
        bind:this={canvas}
        style:width="{box.width}px"
        style:height="{box.height}px"
        aria-hidden="true"
      ></canvas>
      {#if pointer && !space && panning === null}
        <span
          class="cursor"
          style:width="{brush.size * view.zoom}px"
          style:height="{brush.size * view.zoom}px"
          style:left="{pointer.x}px"
          style:top="{pointer.y}px"
        ></span>
      {/if}
      {#if !session}
        <p class="status">{failed ? t("liquify.error") : t("liquify.loading")}</p>
      {/if}
    </div>

    <aside class="settings">
      <h3>{t("liquify.brush")}</h3>
      <div class="sliders">
        <SliderField
          label={t("liquify.size")}
          bind:value={brush.size}
          min={BRUSH_LIMITS.size.min}
          max={BRUSH_LIMITS.size.max}
          unit="px"
          width={60}
          log
        />
        <SliderField
          label={t("liquify.density")}
          bind:value={brush.density}
          min={BRUSH_LIMITS.density.min}
          max={BRUSH_LIMITS.density.max}
        />
        <SliderField
          label={t("liquify.pressure")}
          bind:value={brush.pressure}
          min={BRUSH_LIMITS.pressure.min}
          max={BRUSH_LIMITS.pressure.max}
        />
        <SliderField
          label={t("liquify.rate")}
          bind:value={brush.rate}
          min={BRUSH_LIMITS.rate.min}
          max={BRUSH_LIMITS.rate.max}
        />
      </div>
      <button
        type="button"
        class="btn"
        disabled={!session?.displaced}
        onclick={() => void restoreAll()}
      >
        {t("liquify.restoreAll")}
      </button>
      <label class="check">
        <input type="checkbox" bind:checked={showFrozen} />
        {t("liquify.showFrozen")}
      </label>
      <div class="history">
        <button
          type="button"
          class="btn small"
          disabled={!session?.canUndo}
          onclick={() => void undo(false)}
        >
          {t("liquify.undo")}
        </button>
        <button
          type="button"
          class="btn small"
          disabled={!session?.canRedo}
          onclick={() => void undo(true)}
        >
          {t("liquify.redo")}
        </button>
      </div>
    </aside>
  </div>
  <footer>
    <span class="zoom" aria-label={t("liquify.zoom")}>{zoomPercent(view)}</span>
    <span class="spacer"></span>
    <button
      type="button"
      class="btn primary"
      disabled={!session || failed}
      onclick={() => void ok()}
    >
      {t("sizeDialog.ok")}
    </button>
    <button type="button" class="btn" onclick={cancel}>{t("sizeDialog.cancel")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(96vw, 1400px);
    height: min(92vh, 900px);
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  dialog[open] {
    display: grid;
    grid-template-rows: auto 1fr auto;
  }

  dialog::backdrop {
    background: #0008;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .body {
    display: grid;
    grid-template-columns: auto 1fr 240px;
    min-height: 0;
  }

  .tools {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 6px;
    background: var(--chrome);
    border-right: 1px solid var(--border-dark);
  }

  .tool {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 3px;
    background: none;
    color: var(--text);
  }

  .tool:hover {
    background: var(--hover);
  }

  .tool.on {
    background: var(--selected);
    border-color: var(--accent);
  }

  /* Transparent pixels show over a checkerboard. */
  .stage {
    position: relative;
    outline: none;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: repeating-conic-gradient(#3a3a3a 0 25%, #2c2c2c 0 50%) 0 0 / 16px 16px;
    cursor: none;
    touch-action: none;
  }

  .stage.panning {
    cursor: grab;
  }

  canvas {
    display: block;
  }

  /* The brush's outline, centered on the pointer. */
  .cursor {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid #fff;
    border-radius: 50%;
    box-shadow:
      0 0 0 1px #000a,
      inset 0 0 0 1px #000a;
    transform: translate(-50%, -50%);
    pointer-events: none;
  }

  .status {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
    color: var(--text-muted);
  }

  .settings {
    display: grid;
    align-content: start;
    gap: 10px;
    padding: 10px;
    border-left: 1px solid var(--border-dark);
    overflow: auto;
  }

  h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .sliders {
    display: grid;
    gap: 8px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .history {
    display: flex;
    gap: 6px;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }

  .spacer {
    flex: 1;
  }

  .zoom {
    color: var(--text-muted);
    min-width: 5ch;
  }

  footer .btn {
    min-width: 80px;
  }
</style>
