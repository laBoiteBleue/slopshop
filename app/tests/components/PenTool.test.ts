import { render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import PenTool from "../../src/lib/PenTool.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

function open() {
  const onpath = vi.fn();
  const result = render(PenTool, { mapping: MAPPING, onpath });
  const svg = result.container.querySelector("svg") as SVGSVGElement;
  return { onpath, svg, user: userEvent.setup(), ...result };
}

type User = ReturnType<typeof userEvent.setup>;

const click = (user: User, svg: SVGSVGElement, x: number, y: number) =>
  user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: x, clientY: y } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: x, clientY: y } },
  ]);

test("clicks place corners shown on the image; a click on the first anchor closes the path", async () => {
  const { onpath, svg, user, container } = open();
  await click(user, svg, 10, 10);
  await click(user, svg, 90, 10);
  await click(user, svg, 90, 60);
  expect(container.querySelectorAll("rect.anchor")).toHaveLength(3);
  expect(container.querySelector("rect.anchor.first")).not.toBeNull();
  expect(onpath).not.toHaveBeenCalled();
  await click(user, svg, 12, 12);
  expect(onpath).toHaveBeenCalledWith({
    kind: "path",
    evenOdd: false,
    subpaths: [
      {
        start: [10, 10],
        closed: true,
        segments: [
          { kind: "line", to: [90, 10] },
          { kind: "line", to: [90, 60] },
          { kind: "line", to: [10, 10] },
        ],
      },
    ],
  });
  // The path is done: nothing shown any more.
  expect(container.querySelectorAll("rect.anchor")).toHaveLength(0);
});

test("a drag makes a smooth anchor; Enter ends an open path", async () => {
  const { onpath, svg, user, container } = open();
  await click(user, svg, 0, 0);
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 50, clientY: 0 } },
    { target: svg, coords: { clientX: 70, clientY: 20 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 70, clientY: 20 } },
  ]);
  // Its two handles are shown.
  expect(container.querySelectorAll("circle.handle")).toHaveLength(2);
  await user.keyboard("{Enter}");
  expect(onpath).toHaveBeenCalledWith({
    kind: "path",
    evenOdd: false,
    subpaths: [
      {
        start: [0, 0],
        closed: false,
        segments: [{ kind: "cubic", c1: [0, 0], c2: [30, -20], to: [50, 0] }],
      },
    ],
  });
});

test("Backspace takes back the last anchor; Esc with one anchor left draws nothing", async () => {
  const { onpath, svg, user, container } = open();
  await click(user, svg, 0, 0);
  await click(user, svg, 40, 0);
  await user.keyboard("{Backspace}");
  expect(container.querySelectorAll("rect.anchor")).toHaveLength(1);
  await user.keyboard("{Escape}");
  expect(onpath).not.toHaveBeenCalled();
  expect(container.querySelectorAll("rect.anchor")).toHaveLength(0);
});

test("Shift keeps the next anchor at 45° steps from the last", async () => {
  const { onpath, svg, user } = open();
  await click(user, svg, 0, 0);
  await user.keyboard("{Shift>}");
  await click(user, svg, 100, 4);
  await user.keyboard("{/Shift}{Enter}");
  const geometry = onpath.mock.calls[0][0];
  const end = geometry.subpaths[0].segments[0].to;
  expect(end[0]).toBeCloseTo(100.08, 1);
  expect(end[1]).toBeCloseTo(0, 6);
});

test("changing tool ends the path drawn so far", async () => {
  const { onpath, svg, user, unmount } = open();
  await click(user, svg, 0, 0);
  await click(user, svg, 30, 30);
  unmount();
  expect(onpath).toHaveBeenCalledTimes(1);
});
