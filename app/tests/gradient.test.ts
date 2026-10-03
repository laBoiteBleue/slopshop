import { test } from "vitest";
import assert from "node:assert/strict";
import {
  addStop,
  colorAt,
  cssGradient,
  MAX_STOPS,
  moveStop,
  recolorStop,
  removeStop,
  type Stop,
} from "../src/lib/gradient";

const RED_TO_BLUE: Stop[] = [
  [1024, 255, 0, 0],
  [3072, 0, 0, 255],
];

test("colors between the stops, the end stops' outside", () => {
  assert.deepEqual(colorAt(RED_TO_BLUE, 0), [255, 0, 0]);
  assert.deepEqual(colorAt(RED_TO_BLUE, 2048), [128, 0, 128]);
  assert.deepEqual(colorAt(RED_TO_BLUE, 4096), [0, 0, 255]);
  assert.equal(
    cssGradient(RED_TO_BLUE),
    "linear-gradient(to right, rgb(255, 0, 0) 25.00%, rgb(0, 0, 255) 75.00%)",
  );
});

test("a stop added takes the gradient's color there, in order", () => {
  const added = addStop(RED_TO_BLUE, 2048.4);
  assert.deepEqual(added, {
    stops: [RED_TO_BLUE[0], [2048, 128, 0, 128], RED_TO_BLUE[1]],
    index: 1,
  });
  const full = Array.from({ length: MAX_STOPS }, (_, i): Stop => [i * 200, 0, 0, 0]);
  assert.equal(addStop(full, 100), null);
});

test("a stop moves between its neighbours; two stops stay", () => {
  const three: Stop[] = [
    [0, 0, 0, 0],
    [2000, 9, 9, 9],
    [4096, 255, 255, 255],
  ];
  assert.deepEqual(moveStop(three, 1, 5000)[1], [4096, 9, 9, 9]);
  assert.deepEqual(moveStop(three, 0, -10)[0], [0, 0, 0, 0]);
  assert.deepEqual(moveStop(three, 2, 1000)[2], [2000, 255, 255, 255]);
  assert.deepEqual(removeStop(three, 1), [three[0], three[2]]);
  assert.equal(removeStop(RED_TO_BLUE, 0), null);
  assert.deepEqual(recolorStop(three, 1, [1, 2, 3])[1], [2000, 1, 2, 3]);
});
