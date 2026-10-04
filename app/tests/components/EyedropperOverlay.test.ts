import { fireEvent, render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import EyedropperOverlay from "../../src/lib/EyedropperOverlay.svelte";
import { eyedropperCursor } from "../../src/lib/eyedropper";
import type { EyedropperKind } from "../../src/lib/eyedropper";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 50%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x / 2, y / 2],
  toDocument: (x, y) => [x * 2, y * 2],
  docPerCss: 2,
  hand: false,
};

function open(options: { kind?: EyedropperKind; hand?: boolean } = {}) {
  const onsample = vi.fn();
  // An opaque gray image: the pixels asked around a document point.
  const pixels = vi.fn(async (_x: number, _y: number, radius: number) =>
    new Uint8ClampedArray((2 * radius + 1) ** 2 * 4).fill(128),
  );
  const loupe = { point: MAPPING.toDocument, pixels, version: 1 };
  const { container } = render(EyedropperOverlay, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    kind: options.kind ?? "pick",
    onsample,
    loupe,
  });
  const overlay = container.querySelector(".eyedropper") as HTMLElement;
  return { onsample, pixels, overlay, user: userEvent.setup() };
}

/** jsdom keeps a cursor with a data URL as it was set. */
const cursorOf = (element: HTMLElement) => element.style.cursor;

test("the pointer is the chosen eyedropper; Shift shows the one adding, Alt the one taking away", async () => {
  const { overlay, user } = open({ kind: "subtract" });
  expect(cursorOf(overlay)).toBe(eyedropperCursor("subtract"));
  await user.keyboard("{Shift>}");
  expect(cursorOf(overlay)).toBe(eyedropperCursor("add"));
  await user.keyboard("{/Shift}");
  expect(cursorOf(overlay)).toBe(eyedropperCursor("subtract"));
  const pick = open({ kind: "pick" });
  await pick.user.keyboard("{Alt>}");
  expect(cursorOf(pick.overlay)).toBe(eyedropperCursor("subtract"));
});

test("a click samples the document point under the pointer, with the keys held", async () => {
  const { overlay, onsample, user } = open();
  await user.keyboard("{Shift>}");
  await user.pointer({
    keys: "[MouseLeft]",
    target: overlay,
    coords: { clientX: 30, clientY: 40 },
  });
  expect(onsample).toHaveBeenCalledWith(60, 80, expect.objectContaining({ shiftKey: true }));
});

test("hovering the image shows a loupe of the document pixels around the pointer, which is the pointer then", async () => {
  const { overlay, pixels, user } = open({ kind: "add" });
  await user.pointer({ target: overlay, coords: { clientX: 30, clientY: 40 } });
  await vi.waitFor(() => expect(document.querySelector(".loupe")).toHaveClass("shown"));
  expect(pixels).toHaveBeenCalledWith(60, 80, expect.any(Number));
  expect(cursorOf(overlay)).toBe("none");
  // The eyedropper's sign is on the loupe; Alt shows the one taking away.
  expect(document.querySelector(".loupe .sign")).toHaveTextContent("+");
  await user.keyboard("{Alt>}");
  expect(document.querySelector(".loupe .sign")).toHaveTextContent("−");
  await user.keyboard("{/Alt}");
  await user.unhover(overlay);
  expect(document.querySelector(".loupe")).toBeNull();
  expect(cursorOf(overlay)).toBe(eyedropperCursor("add"));
});

test("while Space pans, neither a click nor the loupe samples", async () => {
  const { overlay, onsample, pixels } = open({ hand: true });
  await fireEvent.pointerMove(overlay, { clientX: 5, clientY: 5 });
  await fireEvent.pointerDown(overlay, { button: 0, clientX: 5, clientY: 5 });
  await new Promise((done) => setTimeout(done));
  expect(onsample).not.toHaveBeenCalled();
  expect(pixels).not.toHaveBeenCalled();
});
