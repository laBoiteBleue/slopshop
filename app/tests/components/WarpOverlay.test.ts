import { render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import WarpOverlay from "../../src/lib/WarpOverlay.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 200 %, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x * 2, y * 2],
  toDocument: (x, y) => [x / 2, y / 2],
  docPerCss: 0.5,
  hand: false,
};

function open() {
  const oncommit = vi.fn();
  const oncancel = vi.fn();
  const lines: [number, number][][] = [
    [
      [0, 0],
      [50, 10],
      [100, 0],
    ],
    [
      [0, 0],
      [0, 50],
    ],
  ];
  const { container } = render(WarpOverlay, { mapping: MAPPING, lines, oncommit, oncancel });
  return { oncommit, oncancel, container };
}

test("the grid's lines are drawn where the view shows them", () => {
  const { container } = open();
  const paths = [...container.querySelectorAll(".warp-grid .line")];
  expect(paths).toHaveLength(2);
  expect(paths[0].getAttribute("d")).toBe("M0.00 0.00 L100.00 20.00 L200.00 0.00");
});

test("Enter applies, Esc cancels, a double-click applies", async () => {
  const { oncommit, oncancel, container } = open();
  const user = userEvent.setup();
  await user.keyboard("{Enter}");
  expect(oncommit).toHaveBeenCalledTimes(1);
  await user.keyboard("{Escape}");
  expect(oncancel).toHaveBeenCalledTimes(1);
  await user.dblClick(container.querySelector(".warp-grid") as Element);
  expect(oncommit).toHaveBeenCalledTimes(2);
});
