<script lang="ts">
  import { engine } from "./engine";
  import { t } from "./i18n/index.svelte";

  let {
    revision,
    onframe,
  }: {
    /** Document revision; a change triggers a new frame. */
    revision: number;
    /** Called with the round-trip time of each displayed frame, in ms. */
    onframe?: (ms: number) => void;
  } = $props();

  let canvas: HTMLCanvasElement;
  /** Viewport size in device pixels: frames are rendered 1:1 with the screen. */
  let size = $state({ width: 0, height: 0 });
  let error = $state<string | null>(null);

  // Latest-wins scheduling: at most one frame in flight; if inputs change meanwhile, render
  // once more when it lands. Keeps the engine from queueing stale work during resizes.
  let inFlight = false;
  let pending = false;

  async function draw() {
    if (inFlight) {
      pending = true;
      return;
    }
    const { width, height } = size;
    if (width === 0 || height === 0) return;
    inFlight = true;
    try {
      const start = performance.now();
      const buffer = await engine.renderView(width, height);
      if (buffer.byteLength !== width * height * 4) {
        throw new Error(`unexpected frame size: ${buffer.byteLength} bytes`);
      }
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      const image = new ImageData(new Uint8ClampedArray(buffer), width, height);
      canvas.getContext("2d")?.putImageData(image, 0, 0);
      error = null;
      onframe?.(performance.now() - start);
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
    // Dependencies: redraw when the document or the viewport size changes.
    void revision;
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
      observer.observe(canvas, { box: "device-pixel-content-box" });
    } catch {
      // WebKit does not support device-pixel-content-box.
      observer.observe(canvas);
    }
    return () => observer.disconnect();
  });
</script>

<div class="viewport">
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
  }

  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
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
