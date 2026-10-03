// Component tests: the DOM matchers, a clean DOM for each test (this project's setup files
// replace the one the `svelteTesting` plugin adds), and what jsdom lacks that the components use.
import "@testing-library/jest-dom/vitest";
import "@testing-library/svelte/vitest";

// Modal dialogs (jsdom/jsdom#3294): opened and closed as a browser does, without the top layer.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
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
