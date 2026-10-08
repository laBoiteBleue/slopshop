import { render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import DirectSelectionTool from "../../src/lib/DirectSelectionTool.svelte";
import type { Matrix } from "../../src/lib/engine";
import type { PenPath } from "../../src/lib/pen";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

/** A corner, then a smooth anchor; the layer placed 100 pixels right. */
const PATHS: PenPath[] = [
  {
    anchors: [
      { point: [10, 10], before: null, after: null },
      { point: [60, 10], before: [40, 10], after: [80, 10] },
    ],
    closed: false,
  },
];
const MATRIX: Matrix = [1, 0, 0, 1, 100, 0];

function open(live = false) {
  const props = {
    mapping: MAPPING,
    paths: PATHS,
    matrix: MATRIX,
    live,
    onedit: vi.fn(),
    oncancel: vi.fn(),
    onconvert: vi.fn(),
  };
  const result = render(DirectSelectionTool, props);
  const svg = result.container.querySelector("svg") as SVGSVGElement;
  return { ...props, svg, user: userEvent.setup(), ...result };
}

test("the anchors are shown where the layer places them; a drag moves one, live", async () => {
  const { svg, user, container, onedit } = open();
  const anchors = container.querySelectorAll("rect.anchor");
  expect(anchors).toHaveLength(2);
  expect(anchors[0].getAttribute("x")).toBe("107");
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 110, clientY: 10 } },
    { target: svg, coords: { clientX: 110, clientY: 40 } },
  ]);
  expect(onedit).toHaveBeenLastCalledWith(
    [
      {
        ...PATHS[0],
        anchors: [{ point: [10, 40], before: null, after: null }, PATHS[0].anchors[1]],
      },
    ],
    false,
  );
  await user.pointer({ keys: "[/MouseLeft]", target: svg, coords: { clientX: 110, clientY: 40 } });
  expect(onedit).toHaveBeenLastCalledWith(expect.any(Array), true);
  expect(onedit).toHaveBeenCalledTimes(2);
});

test("a click selects an anchor and shows its handles; its handle is dragged, Alt alone", async () => {
  const { svg, user, container, onedit } = open();
  expect(container.querySelectorAll("circle.handle")).toHaveLength(0);
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 160, clientY: 10 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 160, clientY: 10 } },
  ]);
  expect(container.querySelectorAll("circle.handle")).toHaveLength(2);
  expect(container.querySelector("rect.anchor.selected")).not.toBeNull();
  await user.keyboard("{Alt>}");
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 180, clientY: 10 } },
    { target: svg, coords: { clientX: 180, clientY: 30 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 180, clientY: 30 } },
  ]);
  await user.keyboard("{/Alt}");
  const [paths, done] = onedit.mock.calls[onedit.mock.calls.length - 1];
  expect(done).toBe(true);
  // The handle after it moved, the one before it kept (Alt).
  expect(paths[0].anchors[1]).toEqual({ point: [60, 10], before: [40, 10], after: [80, 30] });
});

test("a press on a live shape asks to turn it into a path first", async () => {
  const { svg, user, onconvert, onedit } = open(true);
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 110, clientY: 10 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 110, clientY: 10 } },
  ]);
  expect(onconvert).toHaveBeenCalledTimes(1);
  expect(onedit).not.toHaveBeenCalled();
});

test("Esc during a drag gives it up", async () => {
  const { svg, user, oncancel } = open();
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 110, clientY: 10 } },
    { target: svg, coords: { clientX: 130, clientY: 30 } },
  ]);
  await user.keyboard("{Escape}");
  expect(oncancel).toHaveBeenCalledTimes(1);
});
