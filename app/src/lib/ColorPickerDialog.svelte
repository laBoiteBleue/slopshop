<script lang="ts" module>
  /** The component the vertical slider sets; the square sets the two others (Photoshop). */
  export type ColorChannel =
    "hue" | "saturation" | "brightness" | "red" | "green" | "blue" | "lightness" | "labA" | "labB";
</script>

<script lang="ts">
  // The color picker (ADR 0027), as Photoshop's: a square and a vertical slider showing the
  // color space around the color, the slider setting the chosen component (the radio buttons)
  // and the square the two others; HSB, RGB, Lab and hexadecimal fields; the new color over the
  // current one (a click on the current one takes it back). Colors are sRGB; Lab is D50, as
  // Photoshop's. OK applies, Esc or Cancel leaves the color as it was. While it is open the rest
  // of the app waits, except the image: a click or a drag there takes the color shown, as
  // Photoshop's eyedropper does.
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";
  import {
    hexToRgb,
    hsbToRgb,
    labToRgb,
    rgbToHex,
    rgbToHsb,
    rgbToLab,
    type Hsb,
    type Rgb,
  } from "./colorModel";

  let {
    title,
    color,
    onapply,
    onclose,
    sample,
  }: {
    title: string;
    /** The current color, `#rrggbb` sRGB. */
    color: string;
    onapply: (hex: string) => void;
    onclose: () => void;
    /**
     * The color shown in the image under a window point (sRGB in [0, 1]); null where there is
     * no image or nothing shown. `probe` only asks whether there is an image there.
     */
    sample?: {
      at: (clientX: number, clientY: number) => Promise<Rgb | null>;
      probe: (clientX: number, clientY: number) => boolean;
    };
  } = $props();

  const SIDE = 256;
  const initial: Rgb = untrack(() => hexToRgb(color) ?? [0, 0, 0]);
  /** The color is kept as HSB, so that hue survives grays and saturation survives black. */
  let hsb = $state<Hsb>(rgbToHsb(initial));
  let channel = $state<ColorChannel>("hue");
  const rgb = $derived(hsbToRgb(hsb));
  const lab = $derived(rgbToLab(rgb));
  const hex = $derived(rgbToHex(rgb));

  let dialog: HTMLDialogElement;
  let square: HTMLCanvasElement;
  let slider: HTMLCanvasElement;

  function setRgb(next: Rgb) {
    hsb = rgbToHsb(next, hsb[0]);
  }

  /** Each channel: its range, and how the square's axes and the slider map to colors. */
  type Axis = { min: number; max: number };
  const RANGES: Record<ColorChannel, Axis> = {
    hue: { min: 0, max: 360 },
    saturation: { min: 0, max: 1 },
    brightness: { min: 0, max: 1 },
    red: { min: 0, max: 1 },
    green: { min: 0, max: 1 },
    blue: { min: 0, max: 1 },
    lightness: { min: 0, max: 100 },
    labA: { min: -128, max: 127 },
    labB: { min: -128, max: 127 },
  };

  /** The square's horizontal and vertical channels for `channel` on the slider. */
  const PLANES: Record<ColorChannel, [ColorChannel, ColorChannel]> = {
    hue: ["saturation", "brightness"],
    saturation: ["hue", "brightness"],
    brightness: ["hue", "saturation"],
    red: ["blue", "green"],
    green: ["blue", "red"],
    blue: ["red", "green"],
    lightness: ["labA", "labB"],
    labA: ["labB", "lightness"],
    labB: ["labA", "lightness"],
  };

  const FAMILY: Record<ColorChannel, "hsb" | "rgb" | "lab"> = {
    hue: "hsb",
    saturation: "hsb",
    brightness: "hsb",
    red: "rgb",
    green: "rgb",
    blue: "rgb",
    lightness: "lab",
    labA: "lab",
    labB: "lab",
  };

  const INDEX: Record<ColorChannel, number> = {
    hue: 0,
    saturation: 1,
    brightness: 2,
    red: 0,
    green: 1,
    blue: 2,
    lightness: 0,
    labA: 1,
    labB: 2,
  };

  /** Value of `c` in the current color. */
  function valueOf(c: ColorChannel): number {
    const family = FAMILY[c];
    const v = family === "hsb" ? hsb : family === "rgb" ? rgb : lab;
    return v[INDEX[c]];
  }

  /** The color with the channels of `values` (all of one family) changed. */
  function colorWith(values: Partial<Record<ColorChannel, number>>): Rgb {
    const channels = Object.keys(values) as ColorChannel[];
    const family = FAMILY[channels[0]];
    const base: number[] = family === "hsb" ? [...hsb] : family === "rgb" ? [...rgb] : [...lab];
    for (const c of channels) base[INDEX[c]] = values[c] as number;
    if (family === "hsb") return hsbToRgb(base as Hsb);
    if (family === "rgb") return base as Rgb;
    return labToRgb(base as [number, number, number]).rgb;
  }

  function at(axis: Axis, t: number): number {
    return axis.min + (axis.max - axis.min) * t;
  }

  function fraction(c: ColorChannel): number {
    const axis = RANGES[c];
    return (valueOf(c) - axis.min) / (axis.max - axis.min);
  }

  /** Draw the square and the slider for the current color and channel. */
  function draw() {
    const [hx, vy] = PLANES[channel];
    const fixed = valueOf(channel);
    const squareContext = square.getContext("2d");
    const sliderContext = slider.getContext("2d");
    if (!squareContext || !sliderContext) return;
    const image = squareContext.createImageData(SIDE, SIDE);
    for (let y = 0; y < SIDE; y++) {
      for (let x = 0; x < SIDE; x++) {
        const c = colorWith({
          [channel]: fixed,
          [hx]: at(RANGES[hx], x / (SIDE - 1)),
          [vy]: at(RANGES[vy], 1 - y / (SIDE - 1)),
        });
        const i = (y * SIDE + x) * 4;
        image.data[i] = c[0] * 255;
        image.data[i + 1] = c[1] * 255;
        image.data[i + 2] = c[2] * 255;
        image.data[i + 3] = 255;
      }
    }
    squareContext.putImageData(image, 0, 0);
    const strip = sliderContext.createImageData(1, SIDE);
    for (let y = 0; y < SIDE; y++) {
      const value = at(RANGES[channel], 1 - y / (SIDE - 1));
      // The hue slider shows the full rainbow, as Photoshop's.
      const c = channel === "hue" ? hsbToRgb([value, 1, 1]) : colorWith({ [channel]: value });
      strip.data[y * 4] = c[0] * 255;
      strip.data[y * 4 + 1] = c[1] * 255;
      strip.data[y * 4 + 2] = c[2] * 255;
      strip.data[y * 4 + 3] = 255;
    }
    sliderContext.putImageData(strip, 0, 0);
  }

  $effect(() => {
    void hsb;
    void channel;
    draw();
  });

  /** A drag in the square or on the slider sets the color as the pointer moves. */
  function drag(e: PointerEvent, kind: "square" | "slider") {
    if (e.buttons !== 1) return;
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const fx = Math.min(Math.max((e.clientX - box.left) / box.width, 0), 1);
    const fy = Math.min(Math.max((e.clientY - box.top) / box.height, 0), 1);
    if (kind === "square") {
      const [hx, vy] = PLANES[channel];
      setFromChannels({ [hx]: at(RANGES[hx], fx), [vy]: at(RANGES[vy], 1 - fy) });
    } else {
      setFromChannels({ [channel]: at(RANGES[channel], 1 - fy) });
    }
  }

  /** Set `values`, channels of one family (HSB directly: hue survives grays). */
  function setChannels(values: Partial<Record<ColorChannel, number>>) {
    const channels = Object.keys(values) as ColorChannel[];
    if (FAMILY[channels[0]] === "hsb") {
      const next: Hsb = [...hsb];
      for (const c of channels) next[INDEX[c]] = values[c] as number;
      hsb = next;
    } else {
      setRgb(colorWith(values));
    }
  }

  /** Set channels of the square or the slider, the slider's channel kept as it is. */
  function setFromChannels(values: Partial<Record<ColorChannel, number>>) {
    setChannels({ [channel]: valueOf(channel), ...values });
  }

  /** The fields: label, channel, shown scale (degrees, percents, bytes) and unit. */
  const FIELDS: { channel: ColorChannel; label: MessageKey; scale: number; unit: string }[] = [
    { channel: "hue", label: "colorPicker.h", scale: 1, unit: "°" },
    { channel: "saturation", label: "colorPicker.s", scale: 100, unit: "%" },
    { channel: "brightness", label: "colorPicker.b", scale: 100, unit: "%" },
    { channel: "red", label: "colorPicker.r", scale: 255, unit: "" },
    { channel: "green", label: "colorPicker.g", scale: 255, unit: "" },
    { channel: "blue", label: "colorPicker.blue", scale: 255, unit: "" },
    { channel: "lightness", label: "colorPicker.l", scale: 1, unit: "" },
    { channel: "labA", label: "colorPicker.a", scale: 1, unit: "" },
    { channel: "labB", label: "colorPicker.labB", scale: 1, unit: "" },
  ];

  function typed(c: ColorChannel, scale: number, v: number) {
    if (!Number.isFinite(v)) return;
    const axis = RANGES[c];
    // A field sets its own channel, whatever the slider shows.
    setChannels({ [c]: Math.min(Math.max(v / scale, axis.min), axis.max) });
  }

  /** The pointer is over the image: the eyedropper shows. */
  let overImage = $state(false);
  /** A press on the image samples until released; one sample in flight at a time. */
  let sampling: { pointerId: number; busy: boolean; next: [number, number] | null } | null = null;

  async function takeSample(x: number, y: number) {
    const run = sampling;
    if (!sample || !run) return;
    if (run.busy) {
      run.next = [x, y];
      return;
    }
    run.busy = true;
    try {
      const shown = await sample.at(x, y);
      if (shown) setRgb(shown);
    } catch {
      // Nothing to take there.
    } finally {
      run.busy = false;
      const next = run.next;
      run.next = null;
      if (next && sampling === run) void takeSample(...next);
    }
  }

  function onBlockerDown(e: PointerEvent) {
    e.preventDefault();
    if (e.button !== 0 || !sample?.probe(e.clientX, e.clientY)) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    sampling = { pointerId: e.pointerId, busy: false, next: null };
    void takeSample(e.clientX, e.clientY);
  }

  function onBlockerMove(e: PointerEvent) {
    overImage = sample?.probe(e.clientX, e.clientY) ?? false;
    if (sampling?.pointerId === e.pointerId && overImage) void takeSample(e.clientX, e.clientY);
  }

  function onBlockerUp(e: PointerEvent) {
    if (sampling?.pointerId === e.pointerId) sampling = null;
  }

  onMount(() => {
    // Not modal for the browser, so that the image can be sampled: the blocker keeps the rest
    // of the app waiting, and the app's shortcuts must not act behind the dialog.
    dialog.show();
    const keys = (e: KeyboardEvent) => {
      e.stopPropagation();
      if (e.key === "Escape") {
        e.preventDefault();
        onclose();
      } else if (e.key === "Enter" && !dialog.contains(e.target as Node)) {
        e.preventDefault();
        onapply(hex);
      }
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });
</script>

<div
  class="blocker"
  class:eyedropper={overImage}
  role="presentation"
  onpointerdown={onBlockerDown}
  onpointermove={onBlockerMove}
  onpointerup={onBlockerUp}
  onpointercancel={onBlockerUp}
  onpointerleave={() => (overImage = false)}
  oncontextmenu={(e) => e.preventDefault()}
></div>

<dialog bind:this={dialog} aria-labelledby="color-picker-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      onapply(hex);
    }}
  >
    <header id="color-picker-title">{title}</header>
    <div class="body">
      <div class="square-wrap">
        <canvas
          class="square"
          width={SIDE}
          height={SIDE}
          bind:this={square}
          onpointerdown={(e) => {
            (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
            drag(e, "square");
          }}
          onpointermove={(e) => drag(e, "square")}
        ></canvas>
        <span
          class="marker"
          style:left="{fraction(PLANES[channel][0]) * 100}%"
          style:top="{(1 - fraction(PLANES[channel][1])) * 100}%"
        ></span>
      </div>
      <div class="slider-wrap">
        <canvas
          class="slider"
          width="1"
          height={SIDE}
          bind:this={slider}
          onpointerdown={(e) => {
            (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
            drag(e, "slider");
          }}
          onpointermove={(e) => drag(e, "slider")}
        ></canvas>
        <span class="arrows" style:top="{(1 - fraction(channel)) * 100}%"></span>
      </div>
      <div class="side">
        <div class="compare">
          <span class="caption">{t("colorPicker.new")}</span>
          <span class="swatch" style:background={hex}></span>
          <button
            type="button"
            class="swatch current"
            style:background={rgbToHex(initial)}
            title={t("colorPicker.restore")}
            aria-label={t("colorPicker.restore")}
            onclick={() => (hsb = rgbToHsb(initial))}
          ></button>
          <span class="caption">{t("colorPicker.current")}</span>
        </div>
        <div class="fields">
          {#each FIELDS as field (field.channel)}
            <label class="radio">
              <input type="radio" name="channel" value={field.channel} bind:group={channel} />
              {t(field.label)}
            </label>
            <input
              type="number"
              step="1"
              value={Math.round(valueOf(field.channel) * field.scale)}
              oninput={(e) => typed(field.channel, field.scale, e.currentTarget.valueAsNumber)}
              onchange={(e) =>
                (e.currentTarget.valueAsNumber = Math.round(valueOf(field.channel) * field.scale))}
            />
            <span class="unit">{field.unit}</span>
          {/each}
          <span class="radio">#</span>
          <input
            class="hex"
            type="text"
            spellcheck="false"
            value={hex.slice(1)}
            onchange={(e) => {
              const parsed = hexToRgb(e.currentTarget.value);
              if (parsed) setRgb(parsed);
              e.currentTarget.value = rgbToHex(rgb).slice(1);
            }}
          />
        </div>
      </div>
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary">{t("sizeDialog.ok")}</button>
    </footer>
  </form>
</dialog>

<style>
  /* Above everything but the dialog, menus included. */
  .blocker {
    position: fixed;
    inset: 0;
    z-index: 400;
  }

  .blocker.eyedropper {
    cursor: crosshair;
  }

  dialog {
    position: fixed;
    inset: 0;
    z-index: 401;
    margin: auto;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .body {
    display: flex;
    gap: 12px;
    padding: 12px;
  }

  .square-wrap,
  .slider-wrap {
    position: relative;
    height: 256px;
  }

  .square {
    display: block;
    width: 256px;
    height: 256px;
    border: 1px solid var(--border-dark);
    cursor: crosshair;
  }

  .slider {
    display: block;
    width: 20px;
    height: 256px;
    border: 1px solid var(--border-dark);
    image-rendering: pixelated;
    cursor: ns-resize;
  }

  /* The color's place in the square: a ring visible on light and dark colors. */
  .marker {
    position: absolute;
    width: 10px;
    height: 10px;
    border: 1.5px solid #ffffff;
    border-radius: 50%;
    box-shadow: 0 0 0 1px #000000;
    transform: translate(-50%, -50%);
    pointer-events: none;
  }

  /* The slider's value: arrows on both sides, as Photoshop's. */
  .arrows {
    position: absolute;
    left: -6px;
    right: -6px;
    height: 0;
    pointer-events: none;
  }

  .arrows::before,
  .arrows::after {
    content: "";
    position: absolute;
    top: -5px;
    border: 5px solid transparent;
  }

  .arrows::before {
    left: 0;
    border-left-color: var(--text);
  }

  .arrows::after {
    right: 0;
    border-right-color: var(--text);
  }

  .side {
    display: grid;
    align-content: start;
    gap: 12px;
  }

  .compare {
    display: grid;
    grid-template-columns: 60px;
    justify-items: center;
    gap: 0;
  }

  .caption {
    color: var(--text-muted);
    font-size: 11px;
  }

  .swatch {
    width: 60px;
    height: 30px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 0;
  }

  .swatch.current {
    cursor: pointer;
  }

  .fields {
    display: grid;
    grid-template-columns: auto 52px auto;
    align-items: center;
    gap: 3px 6px;
  }

  .radio {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  .unit {
    color: var(--text-muted);
  }

  .hex {
    grid-column: 2 / 4;
    width: 72px;
    font-family: ui-monospace, monospace;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
