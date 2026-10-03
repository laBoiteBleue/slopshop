import { test } from "vitest";
import assert from "node:assert/strict";
import { pasteUnfit } from "../src/lib/clipboard";

test("pastes are grayed by what the clipboard holds", () => {
  assert.ok(pasteUnfit("nothing", "paste"));
  assert.ok(!pasteUnfit("files", "paste"));
  // In Place needs a place: pixels copied from a document, or layers.
  assert.ok(!pasteUnfit("placed", "pasteInPlace"));
  assert.ok(!pasteUnfit("layers", "pasteInPlace"));
  assert.ok(pasteUnfit("image", "pasteInPlace"));
  // Into needs pixels.
  assert.ok(pasteUnfit("files", "pasteInto"));
  assert.ok(!pasteUnfit("image", "pasteInto"));
});

test("nothing is grayed before the clipboard is known, nor other commands", () => {
  assert.ok(!pasteUnfit(null, "paste"));
  assert.ok(!pasteUnfit("nothing", "copy"));
});
