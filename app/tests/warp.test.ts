import { test } from "vitest";
import assert from "node:assert/strict";
import {
  clampBend,
  DEFAULT_BEND,
  linePath,
  sameWarp,
  settingsOf,
  WARP_STYLES,
  warpRequest,
} from "../src/lib/warp";

test("a layer without a warp starts at None with Photoshop's bend", () => {
  assert.deepEqual(settingsOf({ style: null, bend: 0, vertical: false, lines: [] }), {
    style: null,
    bend: DEFAULT_BEND,
    vertical: false,
  });
  assert.deepEqual(settingsOf({ style: "flag", bend: -0.255, vertical: true, lines: [] }), {
    style: "flag",
    bend: -25.5,
    vertical: true,
  });
});

test("the request sends the bend as a share, None takes the warp away", () => {
  assert.deepEqual(warpRequest(7, { style: "arc", bend: 50, vertical: false }), {
    kind: "warpLayer",
    id: 7,
    style: "arc",
    bend: 0.5,
    vertical: false,
  });
  assert.deepEqual(warpRequest(7, { style: null, bend: 50, vertical: true }), {
    kind: "warpLayer",
    id: 7,
  });
  assert.equal(
    (warpRequest(1, { style: "twist", bend: 250, vertical: false }) as { bend: number }).bend,
    1,
  );
});

test("bends stay within -100 to 100 %", () => {
  assert.equal(clampBend(120), 100);
  assert.equal(clampBend(-130), -100);
  assert.equal(clampBend(Number.NaN), 0);
  assert.equal(clampBend(12.5), 12.5);
});

test("None is None whatever its bend; styles differ by every setting", () => {
  assert.ok(
    sameWarp({ style: null, bend: 50, vertical: false }, { style: null, bend: 0, vertical: true }),
  );
  const arc = { style: "arc" as const, bend: 50, vertical: false };
  assert.ok(sameWarp(arc, { ...arc }));
  assert.ok(!sameWarp(arc, { ...arc, bend: 40 }));
  assert.ok(!sameWarp(arc, { ...arc, vertical: true }));
  assert.ok(!sameWarp(arc, { ...arc, style: "wave" }));
  assert.ok(!sameWarp(arc, { style: null, bend: 50, vertical: false }));
});

test("Photoshop's five styles, in order", () => {
  assert.deepEqual([...WARP_STYLES], ["arc", "bulge", "flag", "wave", "twist"]);
});

test("a line is drawn through its points mapped to the viewport", () => {
  const path = linePath(
    [
      [0, 0],
      [10, 5],
    ],
    (x, y) => [x * 2 + 1, y * 2],
  );
  assert.equal(path, "M1.00 0.00 L21.00 10.00");
});
