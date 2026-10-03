import { test } from "vitest";
import assert from "node:assert/strict";
import { clampPanelWidth, MIN_PANEL_WIDTH } from "../src/lib/panelWidth";

test("the panels keep their minimum width and leave the image room", () => {
  assert.equal(clampPanelWidth(50, 1920), MIN_PANEL_WIDTH);
  assert.equal(clampPanelWidth(400.4, 1920), 400);
  // 1000 - 40 (toolbar) - 240 (image).
  assert.equal(clampPanelWidth(900, 1000), 720);
  // A window too narrow for both: the panels keep their minimum.
  assert.equal(clampPanelWidth(300, 300), MIN_PANEL_WIDTH);
});
