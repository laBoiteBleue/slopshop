import { render } from "@testing-library/svelte";
import { expect, test } from "vitest";
import SmartGuides from "../../src/lib/SmartGuides.svelte";
import type { SmartGuide } from "../../src/lib/snap";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 200%, 10 px from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [10 + x * 2, 10 + y * 2],
  toDocument: (x, y) => [(x - 10) / 2, (y - 10) / 2],
  docPerCss: 0.5,
  hand: false,
};

function lines(guides: SmartGuide[]) {
  const { container } = render(SmartGuides, { guides, mapping: MAPPING });
  return [...container.querySelectorAll("line.guide")].map((line) =>
    ["x1", "y1", "x2", "y2"].map((name) => Number(line.getAttribute(name))),
  );
}

test("a guide is drawn between its document points, at the view's position and scale", () => {
  expect(lines([{ x1: 0, y1: 5, x2: 20, y2: 5 }])).toEqual([[10, 20, 50, 20]]);
});

test("no guides draw nothing", () => {
  expect(lines([])).toEqual([]);
});

test("a measure ends with a short tick at each end, a fixed size on screen", () => {
  const drawn = lines([{ x1: 0, y1: 5, x2: 20, y2: 5, measure: true }]);
  expect(drawn).toEqual([
    [10, 20, 50, 20],
    // Ticks across the line: 4 px either side, whatever the zoom.
    [10, 16, 10, 24],
    [50, 16, 50, 24],
  ]);
});
