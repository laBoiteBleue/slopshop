import { test } from "node:test";
import assert from "node:assert/strict";
import { exportFileName, formatOfPath, formatOrder, isVectorPath } from "../src/lib/fileNames.ts";

const FORMATS = {
  png: { extensions: ["png"] },
  jpeg: { extensions: ["jpg", "jpeg"] },
  tiff: { extensions: ["tif", "tiff"] },
};

test("a path's format by its extension, in any case", () => {
  assert.equal(formatOfPath("C:\\photos\\cat.JPEG", FORMATS), "jpeg");
  assert.equal(formatOfPath("/a.b/cat.tif", FORMATS), "tiff");
  assert.equal(formatOfPath("cat.webp", FORMATS), null);
  assert.equal(formatOfPath("cat", FORMATS), null);
});

test("the last format used comes first", () => {
  assert.deepEqual(formatOrder(["png", "jpeg", "tiff"], "tiff"), ["tiff", "png", "jpeg"]);
  assert.deepEqual(formatOrder(["png", "jpeg", "tiff"], "png"), ["png", "jpeg", "tiff"]);
});

test("an export is named after the document, without forbidden characters", () => {
  assert.equal(exportFileName("cat.jpg", "png"), "cat.png");
  assert.equal(exportFileName("Untitled-1", "png"), "Untitled-1.png");
  assert.equal(exportFileName('a/b:c*d?"e<f>g|h', "tif"), "a_b_c_d__e_f_g_h.tif");
  assert.equal(exportFileName("archive.tar.gz", "png"), "archive.tar.png");
  assert.equal(exportFileName(".hidden", "png"), ".hidden.png");
});

test("PDF and SVG open as vectors", () => {
  assert.ok(isVectorPath("a.PDF") && isVectorPath("a.svg") && isVectorPath("a.svgz"));
  assert.ok(!isVectorPath("a.png") && !isVectorPath("a.svg.png"));
});
