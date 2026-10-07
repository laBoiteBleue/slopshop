import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import DocumentInfoDialog from "../../src/lib/DocumentInfoDialog.svelte";
import type { DocumentInfo } from "../../src/lib/engine";

/** A 3000 × 2000 px document at 300 ppi, one pixel layer, in memory only. */
function info(changes: Partial<DocumentInfo> = {}): DocumentInfo {
  return {
    name: "Poster",
    width: 3000,
    height: 2000,
    workingSpace: "srgb",
    blendSpace: "perceptual",
    resolution: 300,
    layers: { raster: 1, fill: 0, vector: 0, adjustment: 0, group: 0, masks: 0 },
    formats: [{ bits: 8, float: false, channels: "rgba", space: "srgb", layers: 1 }],
    memoryBytes: 24 * 1024 * 1024,
    source: null,
    file: null,
    ...changes,
  };
}

function open(changes: Partial<DocumentInfo> = {}) {
  const onclose = vi.fn();
  render(DocumentInfoDialog, { info: info(changes), onclose });
  return { onclose, user: userEvent.setup() };
}

/** What the dialog says for a term. */
const definition = (term: string) =>
  screen.getByText(term, { selector: "dt" }).nextElementSibling as HTMLElement;

test("it says what the document is made of, with sizes in the user's units", () => {
  open();
  expect(screen.getByRole("dialog", { name: "Document Info" })).toBeInTheDocument();
  expect(definition("Name")).toHaveTextContent("Poster");
  expect(definition("Dimensions")).toHaveTextContent("3,000 × 2,000 px (6 megapixels)");
  expect(definition("Resolution")).toHaveTextContent("300 ppi (prints at 25.4 × 16.9 cm)");
  expect(definition("Color space")).toHaveTextContent("sRGB");
  expect(definition("Blending")).toHaveTextContent("Perceptual");
  expect(definition("Memory")).toHaveTextContent("24 MB");
});

test("an unnamed document is Untitled, and only the kinds of layers present are counted", () => {
  open({
    name: null,
    layers: { raster: 3, fill: 0, vector: 4, adjustment: 2, group: 0, masks: 1 },
    blendSpace: "linear",
  });
  expect(definition("Name")).toHaveTextContent("Untitled");
  expect(definition("Pixel layers")).toHaveTextContent("3");
  expect(definition("Vector layers")).toHaveTextContent("4");
  expect(definition("Adjustment layers")).toHaveTextContent("2");
  expect(definition("Layer masks")).toHaveTextContent("1");
  expect(screen.queryByText("Fill layers")).not.toBeInTheDocument();
  expect(screen.queryByText("Groups")).not.toBeInTheDocument();
  expect(definition("Blending")).toHaveTextContent("Linear");
});

test("the pixel formats are listed with how many layers use each", () => {
  open({
    formats: [
      { bits: 16, float: false, channels: "rgb", space: "display-p3", layers: 2 },
      { bits: 32, float: true, channels: "grayAlpha", space: "linear-srgb", layers: 1 },
    ],
  });
  const formats = definition("Pixel formats");
  expect(formats).toHaveTextContent("16-bit RGB · Display P3 × 2");
  expect(formats).toHaveTextContent("32-bit float Grayscale + alpha · Linear sRGB × 1");
});

test("the files it comes from show, with their size or that they are gone", () => {
  open({
    source: { path: "C:\\photos\\poster.jpg", bytes: 5 * 1024 * 1024 },
    file: { path: "C:\\work\\poster.slop", bytes: null },
  });
  expect(definition("Opened from")).toHaveTextContent("C:\\photos\\poster.jpg (5 MB)");
  expect(definition("Document file")).toHaveTextContent("C:\\work\\poster.slop (not found)");
});

test("a document saved where it was opened from shows that file once", () => {
  open({
    source: { path: "C:\\work\\poster.slop", bytes: 1024 * 1024 },
    file: { path: "C:\\work\\poster.slop", bytes: 1024 * 1024 },
  });
  expect(screen.getByText("Opened from")).toBeInTheDocument();
  expect(screen.queryByText("Document file")).not.toBeInTheDocument();
});

test("OK and Escape close it, and the app's keys wait meanwhile", async () => {
  const { onclose, user } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.keyboard("v");
  window.removeEventListener("keydown", appKeys);
  expect(appKeys).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "OK" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
});
