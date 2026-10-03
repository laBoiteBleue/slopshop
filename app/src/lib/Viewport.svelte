<script lang="ts" module>
  import type { Guide } from "./snap";

  /** How an overlay (e.g. Free Transform's box) maps between the document and the viewport. */
  export type ViewMapping = {
    /** Where a document point is in the viewport, in CSS pixels. */
    toViewport: (x: number, y: number) => [number, number];
    /** The document point under a window point (`clientX`, `clientY`). */
    toDocument: (clientX: number, clientY: number) => [number, number];
    /** Document pixels per CSS pixel. */
    docPerCss: number;
    /** Space is held: a drag pans (the hand tool), overlays let it through. */
    hand: boolean;
  };

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
  import { untrack, type Snippet } from "svelte";
  import { engine, type DeviceRect, type ViewInfo, type ViewRequest } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { hasShortcutModifier } from "./platform";

  let {
    documentId,
    revision,
    quickMask = false,
    native = false,
    onframe,
    onmovestart,
    onmove,
    onmoveend,
    ondoubleclick,
    guides = [],
    overlay,
  }: {
    /** Open document. Read once: the viewport is recreated for another document. */
    documentId: number;
    /** Document revision; a change triggers a new frame. */
    revision: number;
    /** Quick Mask over the image (drawn by the engine); a change triggers a new frame. */
    quickMask?: boolean;
    /**
     * Native presentation: the engine presents to the window under this (transparent) area
     * instead of sending frames. Read once, like the document.
     */
    native?: boolean;
    onframe?: (stats: FrameStats) => void;
    /**
     * The Move tool (ADR 0017): a left drag on the image starts at document point (`x`, `y`)
     * (`ctrl`: Ctrl or Cmd held), moves by (`dx`, `dy`) document pixels (fractions: the owner
     * rounds; `docPerCss`: document pixels per CSS pixel, for snapping distances; `free`: Ctrl
     * held, no snapping), then ends.
     */
    onmovestart?: (x: number, y: number, ctrl: boolean, alt: boolean) => void;
    onmove?: (dx: number, dy: number, docPerCss: number, free: boolean) => void;
    onmoveend?: () => void;
    /** A double-click on the image with the Move tool (Free Transform, as in Photoshop). */
    ondoubleclick?: () => void;
    /** Smart guides to draw over the image, in document pixels. */
    guides?: Guide[];
    /** Drawn over the image, following the view (it handles its own pointer events). */
    overlay?: Snippet<[ViewMapping]>;
  } = $props();

  // One viewport per document (it is keyed by document): capture the id, since props are
  // read lazily and work still in flight after a tab switch must not target the new document.
  // Never depend on the prop in effects either: it is a getter on the parent's document view,
  // which is a new object after every edit, so the effect would rerun on each edit.
  const docId = untrack(() => documentId);
  const presentsNatively = untrack(() => native);
  // Same for the revision: only a new value (a derived compares) may trigger a new frame.
  const currentRevision = $derived(revision);
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
  /** `target`, for what follows the view on screen (overlays). */
  let targetView = $state<View | null>(null);
  /** View requests sent but not answered yet. */
  let viewRequestsInFlight = 0;
  /** View answers received so far: tells whether the view moved while a frame rendered. */
  let viewResponses = 0;

  function applyReprojection() {
    if (presentsNatively) return;
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

  // --- Frames --------------------------------------------------------------------------------

  // Latest-wins scheduling: at most one frame in flight; if inputs change meanwhile, render
  // once more when it lands. Keeps the engine from queueing stale work.
  let inFlight = false;
  let pending = false;

  /** Canvas area in physical pixels of the window's client area. */
  function deviceRect(): DeviceRect {
    const rect = container.getBoundingClientRect();
    const dpr = window.devicePixelRatio;
    return {
      x: Math.round(rect.left * dpr),
      y: Math.round(rect.top * dpr),
      width: size.width,
      height: size.height,
    };
  }

  /** Native presents skipped in a row (occluded or busy swapchain), retried a few times. */
  let skippedPresents = 0;

  async function present() {
    const start = performance.now();
    const responsesAtStart = viewResponses;
    const info = await engine.presentView(docId, deviceRect());
    if (destroyed) return;
    error = null;
    // As frames do: the view presented is the latest one if it did not move meanwhile. Without
    // it, the first view (fitted at opening) stayed unknown until a pan or zoom, and with it
    // the overlays (tools, marching ants).
    if (viewResponses === responsesAtStart && viewRequestsInFlight === 0 && !animating) {
      target = { zoom: info.zoom, origin: info.origin };
      targetView = target;
    }
    report({
      zoom: info.zoom,
      fit: info.fit,
      renderMs: info.renderMs,
      totalMs: performance.now() - start,
    });
    if (info.presented) {
      skippedPresents = 0;
      // Shown partly from a coarser level while the rest is composited: refine on the next
      // animation frame (each present composites a bounded amount, so the UI stays responsive).
      if (!info.complete) pending = true;
    } else if (skippedPresents++ < 3) {
      pending = true;
    }
  }

  async function draw() {
    if (destroyed) return;
    if (inFlight) {
      pending = true;
      return;
    }
    const { width, height } = size;
    if (width === 0 || height === 0) return;
    inFlight = true;
    if (presentsNatively) {
      try {
        await present();
      } catch (e) {
        error = String(e);
      } finally {
        inFlight = false;
        if (pending) {
          pending = false;
          requestAnimationFrame(() => void draw());
        }
      }
      return;
    }
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
        targetView = target;
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
    // Dependencies: redraw when the document content, the view, its overlays or the viewport
    // size changes.
    void currentRevision;
    void quickMask;
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
    targetView = target;
    applyReprojection();
    // The zoom readout follows the view immediately, not the next frame.
    if (lastStats) report({ ...lastStats, zoom: info.zoom, fit: info.fit });
    viewEpoch++;
  }

  /** Draw a new frame: what the document shows changed without a new revision (a paint
   * stroke's preview, ADR 0027). */
  export function redraw() {
    void draw();
  }

  /** Zoom about the viewport center (zoom slider). Replaces any wheel zoom still animating. */
  export function zoomTo(zoom: number): Promise<void> {
    remainingLogZoom = 0;
    return changeView({ kind: "setZoom", zoom });
  }

  /** Show the whole document, and keep doing so on resize. */
  export function fit(): Promise<void> {
    remainingLogZoom = 0;
    return changeView({ kind: "fit" });
  }

  /** Step to the next zoom preset about the viewport center. */
  export function stepZoom(zoomIn: boolean): Promise<void> {
    remainingLogZoom = 0;
    return changeView({ kind: "step", zoomIn, x: null, y: null });
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

  // Move tool: a left drag (without Space) moves the selected layers.
  let moving = $state<{ pointerId: number; x: number; y: number } | null>(null);

  function onPointerDown(e: PointerEvent) {
    const hand = e.button === 1 || (e.button === 0 && spaceHeld);
    if (!hand) {
      if (e.button === 0 && onmove) {
        container.setPointerCapture(e.pointerId);
        moving = { pointerId: e.pointerId, x: e.clientX, y: e.clientY };
        const [x, y] = toDocument(e.clientX, e.clientY);
        onmovestart?.(x, y, hasShortcutModifier(e), e.altKey);
      }
      return;
    }
    e.preventDefault(); // no middle-click autoscroll
    container.setPointerCapture(e.pointerId);
    panning = { pointerId: e.pointerId, x: e.clientX, y: e.clientY };
  }

  function onPointerMove(e: PointerEvent) {
    if (moving && e.pointerId === moving.pointerId) {
      // Output pixels are device pixels; a document pixel is `zoom` of them.
      const scale = window.devicePixelRatio / (target?.zoom ?? 1);
      const dx = (e.clientX - moving.x) * scale;
      const dy = (e.clientY - moving.y) * scale;
      moving = { ...moving, x: e.clientX, y: e.clientY };
      if (dx !== 0 || dy !== 0) onmove?.(dx, dy, scale, hasShortcutModifier(e));
      return;
    }
    if (!panning || e.pointerId !== panning.pointerId) return;
    const dpr = window.devicePixelRatio;
    const dx = (e.clientX - panning.x) * dpr;
    const dy = (e.clientY - panning.y) * dpr;
    panning = { ...panning, x: e.clientX, y: e.clientY };
    if (dx !== 0 || dy !== 0) void changeView({ kind: "pan", dx, dy });
  }

  /** The document point under a window point (CSS pixels); `null` outside the viewport. */
  export function documentPointAt(clientX: number, clientY: number): [number, number] | null {
    const rect = container.getBoundingClientRect();
    const inside =
      clientX >= rect.left && clientY >= rect.top && clientX < rect.right && clientY < rect.bottom;
    return inside ? toDocument(clientX, clientY) : null;
  }

  /** Document coordinates of a point of the window (CSS pixels). */
  function toDocument(clientX: number, clientY: number): [number, number] {
    const rect = container.getBoundingClientRect();
    const dpr = window.devicePixelRatio;
    const view = target ?? { zoom: 1, origin: [0, 0] };
    return [
      view.origin[0] + ((clientX - rect.left) * dpr) / view.zoom,
      view.origin[1] + ((clientY - rect.top) * dpr) / view.zoom,
    ];
  }

  /** Where a document point is in the viewport, in CSS pixels. */
  function toViewport(x: number, y: number): [number, number] {
    const dpr = window.devicePixelRatio;
    const view = target ?? { zoom: 1, origin: [0, 0] };
    return [((x - view.origin[0]) * view.zoom) / dpr, ((y - view.origin[1]) * view.zoom) / dpr];
  }

  const mapping: ViewMapping | null = $derived.by(() => {
    const view = targetView;
    if (!view) return null;
    const dpr = window.devicePixelRatio;
    return {
      toViewport: (x, y) => [
        ((x - view.origin[0]) * view.zoom) / dpr,
        ((y - view.origin[1]) * view.zoom) / dpr,
      ],
      toDocument: (clientX, clientY) => {
        const rect = container.getBoundingClientRect();
        return [
          view.origin[0] + ((clientX - rect.left) * dpr) / view.zoom,
          view.origin[1] + ((clientY - rect.top) * dpr) / view.zoom,
        ];
      },
      docPerCss: dpr / view.zoom,
      hand: spaceHeld,
    };
  });

  function endPan(e: PointerEvent) {
    if (panning && e.pointerId === panning.pointerId) panning = null;
    if (moving && e.pointerId === moving.pointerId) {
      moving = null;
      onmoveend?.();
    }
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
    if (moving) {
      moving = null;
      onmoveend?.();
    }
  }}
/>

<div
  class="viewport"
  class:native={presentsNatively}
  class:hand={spaceHeld}
  class:panning={panning !== null}
  bind:this={container}
  role="presentation"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={endPan}
  onpointercancel={endPan}
  onauxclick={(e) => e.preventDefault()}
  ondblclick={(e) => {
    if (e.button === 0 && onmove && !spaceHeld) ondoubleclick?.();
  }}
>
  <canvas bind:this={canvas} class:hidden={presentsNatively}></canvas>
  {#each guides as guide, i (i)}
    {@const [x1, y1] = toViewport(guide.x1, guide.y1)}
    {@const [x2, y2] = toViewport(guide.x2, guide.y2)}
    <div
      class="guide"
      style:left="{Math.round(Math.min(x1, x2))}px"
      style:top="{Math.round(Math.min(y1, y2))}px"
      style:width="{Math.max(1, Math.round(Math.abs(x2 - x1)))}px"
      style:height="{Math.max(1, Math.round(Math.abs(y2 - y1)))}px"
    ></div>
  {/each}
  {#if overlay && mapping}
    {@render overlay(mapping)}
  {/if}
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

  /* Native presentation: the engine draws this area under the page. */
  .viewport.native {
    background: transparent;
  }

  canvas.hidden {
    display: none;
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

  /* Smart guides, magenta as in Photoshop. */
  .guide {
    position: absolute;
    background: #ff2bd6;
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
