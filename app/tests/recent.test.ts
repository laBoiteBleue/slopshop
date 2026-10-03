import { test } from "vitest";
import assert from "node:assert/strict";
import { baseName, recentLabels } from "../src/lib/recent";

test("a path's last part, with Windows or POSIX separators", () => {
  assert.equal(baseName("C:\\Users\\me\\Pictures\\cat.jpg"), "cat.jpg");
  assert.equal(baseName("/home/me/cat.jpg"), "cat.jpg");
  assert.equal(baseName("C:\\Users\\me\\Pictures\\"), "Pictures");
  assert.equal(baseName("cat.jpg"), "cat.jpg");
});

test("recent entries show their file name, and their folder when two share it", () => {
  assert.deepEqual(
    recentLabels(["C:\\a\\cat.jpg", "C:\\b\\cat.jpg", "C:\\b\\dog.png", "/c/cat.jpg"]),
    ["cat.jpg — a", "cat.jpg — b", "dog.png", "cat.jpg — c"],
  );
});
