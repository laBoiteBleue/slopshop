import { test } from "vitest";
import assert from "node:assert/strict";
import { canvasBounds, cropEdit, outsideCanvas, rotateEdit, sizeEdit } from "../src/lib/imageEdits";

const DOC = { width: 400, height: 300, resolution: 72 };

test("the canvas and the points outside it", () => {
  assert.deepEqual(canvasBounds(DOC), { left: 0, top: 0, right: 400, bottom: 300 });
  assert.ok(!outsideCanvas(DOC, 0, 0));
  assert.ok(!outsideCanvas(DOC, 399.9, 299.9));
  assert.ok(outsideCanvas(DOC, 400, 10));
  assert.ok(outsideCanvas(DOC, -0.1, 10));
});

test("Image Size: a new size or resolution, nothing without a change", () => {
  assert.deepEqual(sizeEdit(DOC, "image", { width: 200, height: 150 }, [0.5, 0.5], 72), {
    request: { kind: "resizeImage", width: 200, height: 150, resolution: 72 },
    resized: true,
  });
  // Only the resolution: no new size to fit on screen.
  assert.deepEqual(sizeEdit(DOC, "image", { width: 400, height: 300 }, [0.5, 0.5], 300), {
    request: { kind: "resizeImage", width: 400, height: 300, resolution: 300 },
    resized: false,
  });
  assert.equal(sizeEdit(DOC, "image", { width: 400, height: 300 }, [0.5, 0.5], 72), null);
});

test("Canvas Size: around the anchor, nothing at the same size", () => {
  assert.deepEqual(sizeEdit(DOC, "canvas", { width: 500, height: 300 }, [0, 1], 72), {
    request: { kind: "canvasSize", width: 500, height: 300, anchor: [0, 1] },
    resized: true,
  });
  assert.equal(sizeEdit(DOC, "canvas", { width: 400, height: 300 }, [0, 1], 300), null);
});

test("a crop to the frame, nothing for the whole canvas", () => {
  assert.deepEqual(cropEdit(DOC, { left: -10, top: 20, right: 300, bottom: 320 }), {
    kind: "crop",
    x: -10,
    y: 20,
    width: 310,
    height: 300,
  });
  assert.equal(cropEdit(DOC, canvasBounds(DOC)), null);
});

test("Arbitrary rotation: clockwise is positive, nothing for whole turns", () => {
  assert.deepEqual(rotateEdit(30, true), { kind: "rotateImageBy", degrees: 30 });
  assert.deepEqual(rotateEdit(30, false), { kind: "rotateImageBy", degrees: -30 });
  // A negative angle typed turns the other way, as in Photoshop.
  assert.deepEqual(rotateEdit(-45, true), { kind: "rotateImageBy", degrees: -45 });
  assert.equal(rotateEdit(0, true), null);
  assert.equal(rotateEdit(360, false), null);
  assert.equal(rotateEdit(Number.NaN, true), null);
});
