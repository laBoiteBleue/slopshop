import { screen } from "@testing-library/svelte";
import { expect, onTestFinished, test, vi } from "vitest";
import { calls, documentView, layer, open, respond, sent } from "./harness";

// The marching ants: drawn by the engine in the natively presented view (ADR 0024), by the SVG
// overlay where frames travel over IPC.

let mode: "window" | "frames" = "window";
respond("presenter_mode", () => mode);
respond("present_view", () => ({
  presented: true,
  complete: true,
  revision: 0,
  zoom: 1,
  origin: [0, 0],
  fit: true,
  renderMs: 1,
}));
respond("selection_outline", () => new Uint32Array([1, 3, 10, 10, 50, 10, 50, 50]).buffer);
respond("set_quick_mask", (args, doc) => ({ ...doc, quickMask: args.on as boolean }));
respond("selection_bounds", () => ({ left: 10, top: 20, right: 110, bottom: 70 }));

/** jsdom lays nothing out: the SVG overlay is told its size. */
function sizeTheOverlay() {
  for (const name of ["clientWidth", "clientHeight"]) {
    Object.defineProperty(HTMLElement.prototype, name, { configurable: true, get: () => 200 });
  }
  onTestFinished(() => {
    for (const name of ["clientWidth", "clientHeight"]) {
      delete (HTMLElement.prototype as unknown as Record<string, unknown>)[name];
    }
  });
}

const cat = (extra: Record<string, unknown> = {}) => ({
  ...documentView(1, "cat.jpg", [layer(1, "Cat")]),
  selectionKey: 7,
  ...extra,
});

/** The ants asked with the latest present. */
const lastAnts = () => sent("present_view").at(-1)?.ants;

test("natively presented, the engine draws the ants: no SVG outline, no outline from the engine", async () => {
  mode = "window";
  sizeTheOverlay();
  open(cat());
  await screen.findByText("cat.jpg");
  await vi.waitFor(() => expect(lastAnts()).toEqual({ matrix: [1, 0, 0, 1, 0, 0], march: true }));
  expect(document.querySelector("path.ants")).toBeNull();
  expect(document.querySelector(".outline")).toBeNull();
  expect(calls.some((c) => c.cmd === "selection_outline")).toBe(false);
});

test("natively presented, no selection: no ants asked", async () => {
  mode = "window";
  open(cat({ selectionKey: null }));
  await screen.findByText("cat.jpg");
  await vi.waitFor(() => expect(sent("present_view")).not.toHaveLength(0));
  expect(lastAnts()).toBeNull();
});

test("natively presented, Quick Mask shows the selection itself: no ants asked", async () => {
  mode = "window";
  const user = open(cat());
  await screen.findByText("cat.jpg");
  await vi.waitFor(() => expect(lastAnts()).not.toBeNull());
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Quick Mask Mode", { selector: ".label" }));
  await vi.waitFor(() => expect(sent("set_quick_mask")).toHaveLength(1));
  await vi.waitFor(() => expect(lastAnts()).toBeNull());
});

test("natively presented, Transform Selection's live matrix moves the ants the engine draws", async () => {
  mode = "window";
  const user = open(cat());
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Transform Selection", { selector: ".label" }));
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(8));
  const svg = document.querySelector(".handle")!.closest("svg")!;
  await user.pointer({ keys: "[MouseRight]", target: svg, coords: { clientX: 5, clientY: 5 } });
  await user.click(screen.getByText("Flip Horizontal"));
  // Flipped about the box's center (x = 60), live: nothing is sent to the selection yet.
  await vi.waitFor(() =>
    expect(lastAnts()).toEqual({ matrix: [-1, 0, 0, 1, 120, 0], march: true }),
  );
  expect(sent("transform_selection")).toEqual([]);
  // Esc: the selection as it was.
  await user.keyboard("{Escape}");
  await vi.waitFor(() => expect(lastAnts()).toEqual({ matrix: [1, 0, 0, 1, 0, 0], march: true }));
});

test("with reduced motion the ants the engine draws stand still", async () => {
  mode = "window";
  vi.stubGlobal("matchMedia", (media: string) => ({
    matches: media === "(prefers-reduced-motion: reduce)",
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  open(cat());
  await screen.findByText("cat.jpg");
  await vi.waitFor(() => expect(lastAnts()).toEqual({ matrix: [1, 0, 0, 1, 0, 0], march: false }));
});

test("with frames over IPC the SVG overlay keeps drawing the ants, and nothing is presented", async () => {
  mode = "frames";
  sizeTheOverlay();
  open(cat());
  await screen.findByText("cat.jpg");
  await vi.waitFor(() =>
    expect(document.querySelector("path.ants")?.getAttribute("d")).toBeTruthy(),
  );
  expect(sent("present_view")).toEqual([]);
  expect(sent("selection_outline")).not.toHaveLength(0);
});
