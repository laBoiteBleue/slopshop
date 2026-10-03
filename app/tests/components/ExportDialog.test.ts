import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import ExportDialog from "../../src/lib/ExportDialog.svelte";
import type { ColorSpaceId, ExportFormat, ExportSpec } from "../../src/lib/engine";

afterEach(() => clearMocks());

/** What the engine starts a PNG export from. */
const PNG: ExportSpec = {
  format: "png",
  sample: "u8",
  compression: "fast",
  quality: null,
  subsampling: null,
  space: "srgb",
  keepAlpha: true,
  matte: [1, 1, 1],
  dither: false,
  gray: false,
};

type Engine = {
  defaults: ExportSpec;
  /** The named spaces the format can store, for color then for gray samples. */
  spaces?: [ColorSpaceId[], ColorSpaceId[]];
  /** The largest side the format can store. */
  maxSide?: number | null;
  /** `export_defaults` fails. */
  failure?: string;
};

function open(engine: Engine, size: [number, number] = [800, 600]) {
  const { defaults, spaces = [["srgb", "display-p3"], ["srgb"]], maxSide = null } = engine;
  mockIPC((cmd, payload) => {
    const args = payload as Record<string, unknown> | undefined;
    if (cmd === "export_defaults") {
      if (engine.failure) throw engine.failure;
      return defaults;
    }
    if (cmd === "export_spaces") return args?.gray ? spaces[1] : spaces[0];
    if (cmd === "export_max_side") return maxSide;
  });
  const onexport = vi.fn();
  const onclose = vi.fn();
  render(ExportDialog, {
    documentId: 4,
    width: size[0],
    height: size[1],
    path: "C:\\out\\poster." + defaults.format,
    format: defaults.format as ExportFormat,
    onexport,
    onclose,
  });
  return { onexport, onclose, user: userEvent.setup() };
}

const exportButton = () => screen.getByRole("button", { name: "Export…" });
/** The settings are there once the engine has answered and Export is enabled. */
const loaded = () => vi.waitFor(() => expect(exportButton()).toBeEnabled());
/** The settings of the last export. */
const exported = (onexport: ReturnType<typeof vi.fn>) => onexport.mock.lastCall?.[2] as ExportSpec;

test("a PNG shows its options from the engine's defaults, and Export sends them with the file", async () => {
  const { onexport, user } = open({ defaults: PNG });
  expect(screen.getByRole("dialog", { name: "Save a copy as PNG" })).toBeInTheDocument();
  expect(screen.getByText("poster.png")).toBeInTheDocument();
  // Nothing can be exported before the settings are known.
  expect(exportButton()).toBeDisabled();
  await loaded();
  expect(screen.getByRole("combobox", { name: "Bit depth" })).toHaveValue("u8");
  expect(screen.getByRole("combobox", { name: "Color space" })).toHaveValue("srgb");
  expect(screen.getByRole("combobox", { name: "Compression" })).toHaveValue("fast");
  expect(screen.getByRole("checkbox", { name: "Keep transparency" })).toBeChecked();
  await user.click(exportButton());
  expect(onexport).toHaveBeenCalledWith(4, "C:\\out\\poster.png", PNG);
});

test("the options changed are what is exported", async () => {
  const { onexport, user } = open({ defaults: PNG });
  await loaded();
  await user.selectOptions(screen.getByRole("combobox", { name: "Bit depth" }), "u16");
  await user.selectOptions(screen.getByRole("combobox", { name: "Color space" }), "display-p3");
  await user.selectOptions(screen.getByRole("combobox", { name: "Compression" }), "small");
  await user.click(exportButton());
  expect(exported(onexport)).toMatchObject({
    sample: "u16",
    space: "display-p3",
    compression: "small",
  });
});

test("dithering is offered for 8-bit samples only", async () => {
  const { user } = open({ defaults: PNG });
  await loaded();
  expect(screen.getByRole("checkbox", { name: "Dither (reduces banding)" })).toBeInTheDocument();
  await user.selectOptions(screen.getByRole("combobox", { name: "Bit depth" }), "u16");
  expect(screen.queryByRole("checkbox", { name: /Dither/ })).not.toBeInTheDocument();
});

test("without transparency, the background color flattens it, and it is exported", async () => {
  const { onexport, user } = open({ defaults: PNG });
  await loaded();
  expect(screen.queryByLabelText("Background")).not.toBeInTheDocument();
  await user.click(screen.getByRole("checkbox", { name: "Keep transparency" }));
  await fireEvent.input(screen.getByLabelText("Background"), { target: { value: "#ff0000" } });
  await user.click(exportButton());
  expect(exported(onexport)).toMatchObject({ keepAlpha: false, matte: [1, 0, 0] });
});

/** What the engine starts a JPEG export from. */
const JPEG: ExportSpec = {
  ...PNG,
  format: "jpeg",
  compression: null,
  quality: 90,
  subsampling: "420",
  keepAlpha: false,
};

test("a JPEG has a quality and a chroma subsampling, which grayscale makes pointless", async () => {
  const { onexport, user } = open({ defaults: JPEG });
  await loaded();
  // One sample type: no choice of depth; no alpha to keep.
  expect(screen.queryByRole("combobox", { name: "Bit depth" })).not.toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "Keep transparency" })).not.toBeInTheDocument();
  expect(screen.getByRole("spinbutton", { name: "Quality" })).toHaveValue(90);
  expect(screen.getByRole("combobox", { name: "Chroma subsampling" })).toHaveValue("420");
  await user.click(screen.getByRole("checkbox", { name: "Grayscale" }));
  expect(screen.queryByRole("combobox", { name: "Chroma subsampling" })).not.toBeInTheDocument();
  await user.click(exportButton());
  expect(exported(onexport)).toMatchObject({ quality: 90, gray: true });
});

test("WebP's lossless mode has no quality, and lossy takes the default one back", async () => {
  const webp: ExportSpec = { ...PNG, format: "webp", compression: "lossy", quality: 60 };
  const { onexport, user } = open({ defaults: webp });
  await loaded();
  expect(screen.getByRole("spinbutton", { name: "Quality" })).toHaveValue(60);
  await user.selectOptions(screen.getByRole("combobox", { name: "Compression" }), "lossless");
  expect(screen.queryByRole("spinbutton", { name: "Quality" })).not.toBeInTheDocument();
  await user.selectOptions(screen.getByRole("combobox", { name: "Compression" }), "lossy");
  expect(screen.getByRole("spinbutton", { name: "Quality" })).toHaveValue(90);
  await user.click(exportButton());
  expect(exported(onexport)).toMatchObject({ compression: "lossy", quality: 90 });
});

test("grayscale keeps the color space if the file can declare it, else goes back to sRGB", async () => {
  const p3: ExportSpec = { ...PNG, space: "display-p3" };
  // Gray PNGs can only be tagged sRGB here.
  const { onexport, user } = open({ defaults: p3 });
  await loaded();
  await user.click(screen.getByRole("checkbox", { name: "Grayscale" }));
  expect(screen.getByRole("combobox", { name: "Color space" })).toHaveValue("srgb");
  await user.click(exportButton());
  expect(exported(onexport)).toMatchObject({ gray: true, space: "srgb" });
});

test("the document's own space is offered only when the defaults picked it", async () => {
  const custom: ExportSpec = { ...PNG, space: "custom" };
  open({ defaults: custom });
  await loaded();
  const space = screen.getByRole("combobox", { name: "Color space" });
  expect(space).toHaveValue("custom");
  expect(
    screen.getByRole("option", { name: "Same as source (embedded profile)" }),
  ).toBeInTheDocument();
});

test("a document too large for the format cannot be exported, and the limit is said", async () => {
  open(
    { defaults: { ...PNG, format: "webp", compression: "lossy", quality: 90 }, maxSide: 16383 },
    [20000, 100],
  );
  expect(
    await screen.findByText(
      "WebP is limited to 16,383 px per side, and this image is 20,000 × 100 px: choose another format.",
    ),
  ).toBeInTheDocument();
  // The settings are there by now: the size alone keeps Export disabled.
  expect(screen.getByRole("combobox", { name: "Color space" })).toBeInTheDocument();
  expect(exportButton()).toBeDisabled();
});

test("settings that cannot be read are reported and nothing can be exported", async () => {
  open({ defaults: PNG, failure: "no such document" });
  expect(
    await screen.findByText("Cannot read the export settings: no such document"),
  ).toBeInTheDocument();
  expect(exportButton()).toBeDisabled();
});

test("Cancel and Escape close it without exporting", async () => {
  const { onexport, onclose, user } = open({ defaults: PNG });
  await loaded();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
  expect(onexport).not.toHaveBeenCalled();
});
