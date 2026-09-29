<script lang="ts" module>
  export type FrameStats = {
    /** 1 = 100%. */
    zoom: number;
    fit: boolean;
    /** Engine render time (GPU + readback), ms. */
    renderMs: number;
    /** Request → pixels on screen, ms. */
    totalMs: number;
  };
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import { engine, type ViewInfo, type ViewRequest } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { hasShortcutModifier } from "./platform";

  let {
    documentId,
    revision,
    onframe,
  }: {
    /** Open document; a change resets the displayed frame. */
    documentId: number;
    /** Document revision; a change triggers a new frame. */
    revision: number;
    onframe?: (stats: FrameStats) => void;
  } = $props();

  // One viewport per document (it is keyed by document): capture the id, since props are
  // read lazily and work still in flight after a tab switch must not target the new document.
  const docId = untrack(() => documentId);
  /** Set when the component is destroyed: in-flight work and animations stop quietly. */
  let destroyed = false;
  $effect(() => () => {
    destroyed = true;
    remainingLogZoom = 0;
  });

  /** Events and coordinates use the container: the canvas itself may be transformed. */
  let container: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  /** Viewport size in device pixels: frames are rendered 1:1 with the screen. */
  let size = $state({ width: 0, height: 0 });
  /** Bumped after every view change (zoom, pan, fit) to request a new frame. */
  let viewEpoch = $state(0);
  let error = $state<string | null>(null);

  // --- Reprojection --------------------------------------------------------------------------
  //
  // Rendering a frame takes a round trip through the engine. Meanwhile the frame on screen is
  // moved and scaled with a CSS transform (composited by the GPU, no new pixels) to match the
  // latest view, so pan and zoom respond immediately and the sharp frame replaces it when it
  // lands. Pure presentation: the view itself is always computed by the engine.

  type View = { zoom: number; origin: [number, number] };
  /** View of the frame currently drawn on the canvas. */
  let shown: View | null = null;
  /** Latest view known from the engine. */
  let target: View | null = null;
  /** View requests sent but not answered yet. */
  let viewRequestsInFlight = 0;
  /** View answers received so far: tells whether the view moved while a frame rendered. */
  let viewResponses = 0;

  function applyReprojection() {
    if (!shown || !target) {
      canvas.style.transform = "";
      return;
    }
    // document = origin + output / zoom, so a point drawn at `output` in the shown frame
    // belongs at (origin_s - origin_t) * zoom_t + output * zoom_t / zoom_s in the target view.
    const k = target.zoom / shown.zoom;
    const dpr = window.devicePixelRatio;
    const tx = ((shown.origin[0] - target.origin[0]) * target.zoom) / dpr;
    const ty = ((shown.origin[1] - target.origin[1]) * target.zoom) / dpr;
    const identity = Math.abs(k - 1) < 1e-9 && Math.abs(tx) < 1e-3 && Math.abs(ty) < 1e-3;
    canvas.style.transform = identity ? "" : `translate(${tx}px, ${ty}px) scale(${k})`;
  }

  $effect(() => {
    // A new document: whatever is on the canvas, and any navigation in progress, belongs to
    // the previous one.
    void documentId;
    shown = null;
    target = null;
    remainingLogZoom = 0;
    panning = null;
    canvas.getContext("2d")?.clearRect(0, 0, canvas.width, canvas.height);
    applyReprojection();
  });

  // --- Frames --------------------------------------------------------------------------------

  // Latest-wins scheduling: at most one frame in flight; if inputs change meanwhile, render
  // once more when it lands. Keeps the engine from queueing stale work.
  let inFlight = false;
  let pending = false;

  async function draw() {
    if (destroyed) return;
    if (inFlight) {
      pending = true;
      return;
    }
    const { width, height } = size;
    if (width === 0 || height === 0) return;
    inFlight = true;
    try {
      const start = performance.now();
      const responsesAtStart = viewResponses;
      const frame = await engine.renderView(docId, width, height);
      if (destroyed) return;
      // A frame of another document (opened while it rendered): drop it. The document change
      // already scheduled a new frame.
      if (frame.documentId !== docId >>> 0) return;
      if (canvas.width !== frame.width || canvas.height !== frame.height) {
        canvas.width = frame.width;
        canvas.height = frame.height;
      }
      const image = new ImageData(frame.pixels, frame.width, frame.height);
      canvas.getContext("2d")?.putImageData(image, 0, 0);
      shown = { zoom: frame.zoom, origin: frame.origin };
      // If the view did not move while the frame rendered, the frame's view is the latest one
      // (e.g. a resize re-fit). Otherwise keep the newer target: the next frame catches up.
      if (viewResponses === responsesAtStart && viewRequestsInFlight === 0 && !animating) {
        target = shown;
      }
      applyReprojection();
      error = null;
      report({
        zoom: target?.zoom ?? frame.zoom,
        fit: frame.fit,
        renderMs: frame.renderMs,
        totalMs: performance.now() - start,
      });
    } catch (e) {
      error = String(e);
    } finally {
      inFlight = false;
      if (pending) {
        pending = false;
        void draw();
      }
    }
  }

  $effect(() => {
    // Dependencies: redraw when the document, the view or the viewport size changes.
    void documentId;
    void revision;
    void viewEpoch;
    void size.width;
    void size.height;
    void draw();
  });

  $effect(() => {
    const observer = new ResizeObserver(([entry]) => {
      const exact = entry.devicePixelContentBoxSize?.[0];
      const dpr = window.devicePixelRatio;
      size = exact
        ? { width: exact.inlineSize, height: exact.blockSize }
        : {
            width: Math.round(entry.contentRect.width * dpr),
            height: Math.round(entry.contentRect.height * dpr),
          };
    });
    try {
      observer.observe(container, { box: "device-pixel-content-box" });
    } catch {
      // WebKit does not support device-pixel-content-box.
      observer.observe(container);
    }
    return () => observer.disconnect();
  });

  // --- View changes ---------------------------------------------------------------------------

  async function changeView(request: ViewRequest) {
    if (destroyed) return;
    viewRequestsInFlight++;
    let info: ViewInfo | null = null;
    try {
      info = await engine.view(docId, request);
    } catch (e) {
      error = String(e);
    } finally {
      viewRequestsInFlight--;
    }
    // `null`: merged into a request still waiting; its answer carries the combined view.
    if (!info || destroyed) return;
    viewResponses++;
    target = { zoom: info.zoom, origin: info.origin };
    applyReprojection();
    // The zoom readout follows the view immediately, not the next frame.
    if (lastStats) report({ ...lastStats, zoom: info.zoom, fit: info.fit });
    viewEpoch++;
  }

  let lastStats: FrameStats | null = null;
  function report(stats: FrameStats) {
    lastStats = stats;
    onframe?.(stats);
  }

  /** Pointer position in viewport device pixels. */
  function devicePoint(e: { clientX: number; clientY: number }): { x: number; y: number } {
    const rect = container.getBoundingClientRect();
    const dpr = window.devicePixelRatio;
    return { x: (e.clientX - rect.left) * dpr, y: (e.clientY - rect.top) * dpr };
  }

  // --- Smooth zoom ------------------------------------------------------------------------------
  //
  // Wheel notches accumulate a zoom target (in log space) that is reached over ~150 ms with an
  // exponential ease, one view request per animation frame, anchored at the pointer.

  const WHEEL_ZOOM_PER_PIXEL = 0.002;
  /** Time constant of the ease, ms: ~95% of a notch is applied after 3τ. */
  const ZOOM_EASE_MS = 50;
  let remainingLogZoom = 0;
  let zoomAnchor = { x: 0, y: 0 };
  let animating = false;
  let lastTick = 0;

  function zoomTick(now: number) {
    if (destroyed) {
      animating = false;
      return;
    }
    // rAF timestamps can precede the wheel handler's performance.now(): never go backwards.
    const dt = Math.max(0, Math.min(now - lastTick, 50));
    lastTick = now;
    let step = remainingLogZoom * (1 - Math.exp(-dt / ZOOM_EASE_MS));
    if (Math.abs(remainingLogZoom - step) < 1e-3) step = remainingLogZoom;
    remainingLogZoom -= step;
    if (step !== 0) {
      void changeView({ kind: "zoomBy", factor: Math.exp(step), ...zoomAnchor });
    }
    if (remainingLogZoom !== 0) {
      requestAnimationFrame(zoomTick);
    } else {
      animating = false;
    }
  }

  function zoomSmoothly(logZoom: number, anchor: { x: number; y: number }) {
    remainingLogZoom += logZoom;
    zoomAnchor = anchor;
    if (!animating) {
      animating = true;
      lastTick = performance.now();
      requestAnimationFrame(zoomTick);
    }
  }

  // --- Input ---------------------------------------------------------------------------------

  /** Vertical wheel delta in CSS pixels, whatever the device reports (pixels, lines, pages). */
  function wheelPixels(e: WheelEvent): number {
    const unit =
      e.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? 16
        : e.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? container.clientHeight
          : 1;
    return e.deltaY * unit;
  }

  // The middle button is dedicated to navigation: the wheel zooms around the pointer (with or
  // without modifiers, so trackpad pinch, reported by Chromium as Ctrl + wheel, zooms too;
  // WebKit pinch uses gesture events, handled below) and a middle-button drag pans.
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const dy = wheelPixels(e);
    if (dy === 0) return;
    zoomSmoothly(-dy * WHEEL_ZOOM_PER_PIXEL, devicePoint(e));
  }

  // WebKit (macOS) reports trackpad pinch as non-standard gesture events with a cumulative
  // `scale`, not as Ctrl + wheel. Pinch is already continuous: applied directly.
  type WebKitGestureEvent = UIEvent & { scale: number; clientX: number; clientY: number };
  let lastGestureScale = 1;

  function onGestureStart(e: Event) {
    e.preventDefault(); // no WebKit page magnification
    lastGestureScale = 1;
  }

  function onGestureChange(e: Event) {
    e.preventDefault();
    const gesture = e as WebKitGestureEvent;
    const factor = gesture.scale / lastGestureScale;
    lastGestureScale = gesture.scale;
    if (!Number.isFinite(factor) || factor === 1) return;
    void changeView({ kind: "zoomBy", factor, ...devicePoint(gesture) });
  }

  function onGestureEnd(e: Event) {
    e.preventDefault();
  }

  $effect(() => {
    // Registered manually: the listeners must not be passive to prevent the page from
    // scrolling or zooming.
    const options = { passive: false };
    container.addEventListener("wheel", onWheel, options);
    container.addEventListener("gesturestart", onGestureStart, options);
    container.addEventListener("gesturechange", onGestureChange, options);
    container.addEventListener("gestureend", onGestureEnd, options);
    return () => {
      container.removeEventListener("wheel", onWheel);
      container.removeEventListener("gesturestart", onGestureStart);
      container.removeEventListener("gesturechange", onGestureChange);
      container.removeEventListener("gestureend", onGestureEnd);
    };
  });

  // Hand tool: drag with the middle button, or hold Space and drag.
  let spaceHeld = $state(false);
  let panning = $state<{ pointerId: number; x: number; y: number } | null>(null);

  function isTextField(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLTextAreaElement ||
      (target instanceof HTMLInputElement && ["text", "number", "search"].includes(target.type))
    );
  }

  function onPointerDown(e: PointerEvent) {
    const hand = e.button === 1 || (e.button === 0 && spaceHeld);
    if (!hand) return;
    e.preventDefault(); // no middle-click autoscroll
    container.setPointerCapture(e.pointerId);
    panning = { pointerId: e.pointerId, x: e.clientX, y: e.clientY };
  }

  function onPointerMove(e: PointerEvent) {
    if (!panning || e.pointerId !== panning.pointerId) return;
    const dpr = window.devicePixelRatio;
    const dx = (e.clientX - panning.x) * dpr;
    const dy = (e.clientY - panning.y) * dpr;
    panning = { ...panning, x: e.clientX, y: e.clientY };
    if (dx !== 0 || dy !== 0) void changeView({ kind: "pan", dx, dy });
  }

  function endPan(e: PointerEvent) {
    if (panning && e.pointerId === panning.pointerId) panning = null;
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (isTextField(e.target)) return;
    if (e.key === " " && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // Space would otherwise press the focused button or scroll.
      e.preventDefault();
      spaceHeld = true;
      return;
    }
    if (!hasShortcutModifier(e) || e.altKey) return;
    let request: ViewRequest | null = null;
    // Digits also match the physical key: on AZERTY the unshifted digit row types "à" and "&".
    if (e.key === "0" || e.code === "Digit0" || e.code === "Numpad0") {
      request = { kind: "fit" };
    } else if (e.key === "1" || e.code === "Digit1" || e.code === "Numpad1") {
      request = { kind: "setZoom", zoom: 1 };
    } else if (e.key === "+" || e.key === "=") {
      request = { kind: "step", zoomIn: true, x: null, y: null };
    } else if (e.key === "-" || e.key === "_") {
      request = { kind: "step", zoomIn: false, x: null, y: null };
    }
    if (request) {
      e.preventDefault();
      // An explicit zoom replaces any wheel zoom still animating.
      remainingLogZoom = 0;
      void changeView(request);
    }
  }

  function onWindowKeyup(e: KeyboardEvent) {
    if (e.key === " ") spaceHeld = false;
  }
</script>

<svelte:window
  onkeydown={onWindowKeydown}
  onkeyup={onWindowKeyup}
  onblur={() => {
    spaceHeld = false;
    panning = null;
  }}
/>

<div
  class="viewport"
  class:hand={spaceHeld}
  class:panning={panning !== null}
  bind:this={container}
  role="presentation"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={endPan}
  onpointercancel={endPan}
  onauxclick={(e) => e.preventDefault()}
>
  <canvas bind:this={canvas}></canvas>
  {#if error}
    <p class="error" role="alert">{t("viewport.renderFailed", { error })}</p>
  {/if}
</div>

<style>
  .viewport {
    position: relative;
    min-width: 0;
    min-height: 0;
    background: var(--pasteboard);
    overflow: hidden;
    touch-action: none;
  }

  .viewport.hand {
    cursor: grab;
  }

  .viewport.panning {
    cursor: grabbing;
  }

  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    transform-origin: 0 0;
    will-change: transform;
    pointer-events: none;
  }

  .error {
    position: absolute;
    left: 12px;
    bottom: 12px;
    margin: 0;
    padding: 6px 10px;
    border-radius: 4px;
    background: var(--danger-bg);
    color: var(--danger-fg);
    font-size: 12px;
  }
</style>
