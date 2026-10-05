import { test } from "vitest";
import assert from "node:assert/strict";
import {
  SLOTS,
  TOOLS,
  isEraser,
  isPaintTool,
  isSelectionTool,
  slotForLetter,
  slotOf,
  slotTool,
  toolInfo,
} from "../src/lib/tools";

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
  assert.deepEqual(
    slotForLetter("i")?.tools.map((tool) => tool.id),
    ["eyedropper"],
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

test("a slot's key picks the tool it shows, Shift the next variant", () => {
  const slot = slotOf("marquee");
  assert.equal(slotTool(slot, undefined, false), "marquee");
  assert.equal(slotTool(slot, "ellipse", false), "ellipse");
  assert.equal(slotTool(slot, "marquee", true), "ellipse");
  assert.equal(slotTool(slot, "ellipse", true), "marquee");
  assert.equal(slotTool(slotOf("move"), undefined, true), "move");
});
