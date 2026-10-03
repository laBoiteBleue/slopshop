import { test } from "node:test";
import assert from "node:assert/strict";
import {
  SLOTS,
  TOOLS,
  isEraser,
  isPaintTool,
  isSelectionTool,
  slotForLetter,
  slotOf,
  toolInfo,
} from "../src/lib/tools.ts";

test("each tool is in one slot, and each slot has its own key", () => {
  const ids = TOOLS.map((tool) => tool.id);
  assert.equal(new Set(ids).size, ids.length);
  const keys = SLOTS.map((slot) => slot.key);
  assert.equal(new Set(keys).size, keys.length);
  for (const tool of TOOLS) assert.ok(slotOf(tool.id).tools.includes(tool));
  for (const tool of TOOLS) assert.equal(toolInfo(tool.id), tool);
});

test("a letter picks its slot, as typed", () => {
  assert.deepEqual(
    slotForLetter("m")?.tools.map((tool) => tool.id),
    ["marquee", "ellipse"],
  );
  assert.equal(slotForLetter("z"), null);
  assert.equal(slotForLetter(null), null);
});

test("tool kinds", () => {
  assert.ok(isPaintTool("brush") && isPaintTool("eraser") && isPaintTool("restoreEraser"));
  assert.ok(isEraser("restoreEraser") && !isEraser("brush"));
  assert.ok(isSelectionTool("wand") && isSelectionTool("polygonalLasso"));
  assert.ok(!isSelectionTool("move") && !isSelectionTool("crop") && !isPaintTool("move"));
});
