import { render } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import Loupe from "../../src/lib/Loupe.svelte";
import { LOUPE_SIDE } from "../../src/lib/eyedropper";
import { reactive } from "./reactive.svelte";

/** A patch of one color, the sampled pixel `middle`. */
function patchOf(middle: [number, number, number, number]) {
  const pixels = new Uint8ClampedArray(LOUPE_SIDE * LOUPE_SIDE * 4).fill(255);
  pixels.set(middle, ((LOUPE_SIDE * LOUPE_SIDE - 1) / 2) * 4);
  return pixels;
}

const loupe = () => document.querySelector(".loupe") as HTMLElement;

test("the loupe shows once the pixels arrive, the sampled one's color on its ring", async () => {
  const patch = vi.fn(async () => patchOf([200, 10, 20, 255]));
  render(Loupe, { x: 500, y: 400, patch });
  expect(patch).toHaveBeenCalledWith(500, 400);
  expect(loupe()).not.toHaveClass("shown");
  await vi.waitFor(() => expect(loupe()).toHaveClass("shown"));
  expect(loupe().style.getPropertyValue("--ring")).toBe("rgb(200 10 20)");
  expect(loupe().style.left).toBe("524px");
});

test("off the image the loupe hides", async () => {
  render(Loupe, { x: 10, y: 10, patch: async () => null });
  await new Promise((done) => setTimeout(done));
  expect(loupe()).not.toHaveClass("shown");
});

test("the loupe asks for one patch at a time, the latest position next", async () => {
  const pending: (() => void)[] = [];
  const patch = vi.fn(
    (_x: number, _y: number) =>
      new Promise<Uint8ClampedArray<ArrayBuffer>>((resolve) =>
        pending.push(() => resolve(patchOf([0, 0, 0, 255]))),
      ),
  );
  const props = reactive({ x: 100, y: 100, patch });
  render(Loupe, props);
  props.x = 110;
  await Promise.resolve();
  props.x = 120;
  await Promise.resolve();
  expect(patch).toHaveBeenCalledTimes(1);
  pending.shift()?.();
  await vi.waitFor(() => expect(patch).toHaveBeenCalledTimes(2));
  expect(patch).toHaveBeenLastCalledWith(120, 100);
});
