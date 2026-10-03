import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import WandTool from "../../src/lib/WandTool.svelte";
import type { SelectionMode } from "../../src/lib/engine";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

function open(options: { mode?: SelectionMode; hand?: boolean } = {}) {
  const onpick = vi.fn();
  const { container } = render(WandTool, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    mode: options.mode ?? "replace",
    onpick,
  });
  return {
    onpick,
    svg: container.querySelector("svg") as SVGSVGElement,
    user: userEvent.setup(),
  };
}

test("a click picks the document pixel under the pointer, with no mode of its own", async () => {
  const { onpick, svg, user } = open();
  await user.pointer({
    keys: "[MouseLeft]",
    target: svg,
    coords: { clientX: 20.7, clientY: 30.2 },
  });
  expect(onpick).toHaveBeenCalledExactlyOnceWith(20, 30, null);
});

test("a click left of the image picks a negative pixel, for the owner to deselect", async () => {
  const { onpick, svg, user } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: -0.5, clientY: 5 } });
  expect(onpick).toHaveBeenCalledExactlyOnceWith(-1, 5, null);
});

test.each([
  ["[ShiftLeft>]", "[/ShiftLeft]", "add"],
  ["[AltLeft>]", "[/AltLeft]", "subtract"],
  ["[ShiftLeft>][AltLeft>]", "[/AltLeft][/ShiftLeft]", "intersect"],
])("keys held at the click (%s) ask for the mode %s", async (down, up, mode) => {
  const { onpick, svg, user } = open();
  await user.keyboard(down);
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: 5, clientY: 5 } });
  await user.keyboard(up);
  expect(onpick).toHaveBeenCalledExactlyOnceWith(5, 5, mode);
});

test("only the left button picks", async () => {
  const { onpick, svg, user } = open();
  await user.pointer({ keys: "[MouseRight]", target: svg, coords: { clientX: 5, clientY: 5 } });
  expect(onpick).not.toHaveBeenCalled();
});

test("while Space pans the viewport, a click picks nothing", async () => {
  const { onpick, svg, user } = open({ hand: true });
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: 5, clientY: 5 } });
  expect(onpick).not.toHaveBeenCalled();
});

test("a badge by the pointer tells the mode the click would take", async () => {
  const { svg, user } = open({ mode: "add" });
  await user.pointer({ target: svg, coords: { clientX: 5, clientY: 5 } });
  expect(screen.getByText("+")).toBeInTheDocument();
  await user.keyboard("[AltLeft>]");
  await user.pointer({ target: svg, coords: { clientX: 6, clientY: 6 } });
  expect(screen.getByText("−")).toBeInTheDocument();
});
