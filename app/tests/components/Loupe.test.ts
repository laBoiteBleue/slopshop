import { render } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import Loupe from "../../src/lib/Loupe.svelte";
import type { LoupeSource } from "../../src/lib/eyedropper";
import { reactive } from "./reactive.svelte";

/** The color shown at document pixel (`x`, `y`): its coordinates, opaque; clear left of x = 50. */
const colorAt = (x: number, y: number) => (x < 50 ? [0, 0, 0, 0] : [x & 255, y & 255, 20, 255]);

/** The pixels around a document pixel, as the engine gives them. */
async function pixelsAround(cx: number, cy: number, radius: number) {
  const side = 2 * radius + 1;
  const pixels = new Uint8ClampedArray(side * side * 4);
  for (let row = 0; row < side; row++) {
    for (let col = 0; col < side; col++) {
      pixels.set(colorAt(cx - radius + col, cy - radius + row), (row * side + col) * 4);
    }
  }
  return pixels;
}

/** The window shows the document at 200% from its corner; the image ends at x = 1000. */
function sourceOf(version: unknown = 1) {
  return {
    point: (x: number, y: number) => (x < 1000 ? ([x * 2, y * 2] as [number, number]) : null),
    pixels: vi.fn(pixelsAround),
    version,
  } satisfies LoupeSource;
}

const loupe = () => document.querySelector(".loupe") as HTMLElement;
const value = () => document.querySelector(".loupe .value");

test("the loupe is centered on the pointer once its pixels arrive, the sampled color on the ring and under it", async () => {
  const source = sourceOf();
  render(Loupe, { x: 100, y: 60, source, current: "#ff0000" });
  expect(loupe()).not.toHaveClass("shown");
  await vi.waitFor(() => expect(loupe()).toHaveClass("shown"));
  expect(source.pixels).toHaveBeenCalledWith(200, 120, 64);
  expect(loupe().style.left).toBe("100px");
  expect(loupe().style.top).toBe("60px");
  // Photoshop's ring: the new color over the current one.
  expect(loupe().style.getPropertyValue("--new")).toBe("#c87814");
  expect(loupe().style.getPropertyValue("--current")).toBe("#ff0000");
  expect(value()).toHaveTextContent("#c87814");
  expect(value()).not.toHaveClass("above");
});

test("it follows the pointer without asking the engine at each move", async () => {
  const source = sourceOf();
  const props = reactive({ x: 100, y: 60, source });
  render(Loupe, props);
  await vi.waitFor(() => expect(loupe()).toHaveClass("shown"));
  props.x = 110;
  await vi.waitFor(() => expect(loupe().style.left).toBe("110px"));
  expect(value()).toHaveTextContent("#dc7814");
  expect(source.pixels).toHaveBeenCalledTimes(1);
  // Without a current color, the new one all round.
  expect(loupe().style.getPropertyValue("--current")).toBe("#dc7814");
});

test("a change of the document asks for its pixels again", async () => {
  const props = reactive({ x: 100, y: 60, source: sourceOf(1) });
  render(Loupe, props);
  await vi.waitFor(() => expect(loupe()).toHaveClass("shown"));
  const next = sourceOf(2);
  props.source = next;
  await vi.waitFor(() => expect(next.pixels).toHaveBeenCalledWith(200, 120, 64));
});

test("off the image the loupe hides, and asks nothing", async () => {
  const source = sourceOf();
  render(Loupe, { x: 1200, y: 60, source });
  await new Promise((done) => requestAnimationFrame(done));
  expect(loupe()).not.toHaveClass("shown");
  expect(source.pixels).not.toHaveBeenCalled();
});

test("a transparent pixel has no value under the loupe", async () => {
  render(Loupe, { x: 10, y: 60, source: sourceOf() });
  await vi.waitFor(() => expect(loupe()).toHaveClass("shown"));
  expect(value()).toBeNull();
});

test("at the bottom of the window the value goes above the loupe", async () => {
  render(Loupe, { x: 100, y: window.innerHeight - 20, source: sourceOf() });
  await vi.waitFor(() => expect(value()).toHaveClass("above"));
});

test("Color Range's adding and subtracting eyedroppers show their sign on the ring", async () => {
  const props = reactive({
    x: 100,
    y: 60,
    source: sourceOf(),
    sign: "add" as "pick" | "add" | "subtract",
  });
  render(Loupe, props);
  const sign = () => document.querySelector(".sign");
  expect(sign()).toHaveTextContent("+");
  props.sign = "subtract";
  await vi.waitFor(() => expect(sign()).toHaveTextContent("−"));
  props.sign = "pick";
  await vi.waitFor(() => expect(sign()).toBeNull());
});
