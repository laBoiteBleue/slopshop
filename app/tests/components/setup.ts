// Component tests: the DOM matchers, a clean DOM for each test (this project's setup files
// replace the one the `svelteTesting` plugin adds), and what jsdom lacks that the components use.
import "@testing-library/jest-dom/vitest";
import "@testing-library/svelte/vitest";

// Dialogs (jsdom/jsdom#3294): opened and closed as a browser does, without the top layer.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};
HTMLDialogElement.prototype.show ??= function (this: HTMLDialogElement) {
  this.open = true;
};
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement, value?: string) {
  if (!this.open) return;
  if (value !== undefined) this.returnValue = value;
  this.open = false;
  this.dispatchEvent(new Event("close"));
};

// Pointer capture: nothing to capture in a simulated DOM.
Element.prototype.setPointerCapture ??= () => {};
Element.prototype.releasePointerCapture ??= () => {};
Element.prototype.hasPointerCapture ??= () => false;

// Visibility observers (lazy thumbnails): nothing scrolls into view in a simulated DOM.
globalThis.IntersectionObserver ??= class {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
} as unknown as typeof IntersectionObserver;

// Canvases: jsdom draws nothing (and complains); the components already handle no context.
HTMLCanvasElement.prototype.getContext = (() =>
  null) as typeof HTMLCanvasElement.prototype.getContext;

// Pixels for a canvas (frames, thumbnails): jsdom has no ImageData without the canvas package.
globalThis.ImageData ??= class {
  readonly colorSpace = "srgb";
  constructor(
    readonly data: Uint8ClampedArray,
    readonly width: number,
    readonly height: number,
  ) {}
} as unknown as typeof ImageData;

// Size observers (`bind:clientWidth`): a browser reports every observed element once at the
// start; nothing is ever laid out in a simulated DOM, so there is never a resize after that.
globalThis.ResizeObserver ??= class {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element) {
    queueMicrotask(() => this.callback([{ target } as ResizeObserverEntry], this));
  }
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

// Range inputs: a browser snaps a value set to the nearest step and keeps it within the range
// (the sanitization algorithm); jsdom keeps it as given, and then finds the form invalid
// (a step mismatch), so that it does not submit. `step="any"` never snaps.
const inputValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
if (inputValue?.set) {
  const set = inputValue.set;
  Object.defineProperty(HTMLInputElement.prototype, "value", {
    ...inputValue,
    set(this: HTMLInputElement, value: string) {
      if (this.type === "range") {
        const min = this.min === "" ? 0 : Number(this.min);
        const max = this.max === "" ? 100 : Number(this.max);
        const step = this.step === "any" ? 0 : Number(this.step) > 0 ? Number(this.step) : 1;
        const given = Number(value);
        if (Number.isFinite(given)) {
          const snapped = step > 0 ? min + Math.round((given - min) / step) * step : given;
          value = String(Math.min(Math.max(snapped, min), max));
        }
      }
      set.call(this, value);
    },
  });
}
