// English catalog: the reference. Every other catalog must provide exactly these keys
// (enforced by the `Messages` type). Placeholders use `{name}`.
const en = {
  "app.preAlpha": "pre-alpha",
  "app.language": "Language",

  "toolbar.undoHint": "Undo ({mod}+Z)",
  "toolbar.redoHint": "Redo ({mod}+Shift+Z)",

  "document.untitled": "Untitled",

  "open.hint": "Open images in new tabs ({mod}+O)",

  "tabs.newHint": "New document ({mod}+N)",
  "tabs.closeHint": "Close ({mod}+W, or middle click)",
  "tabs.hint":
    "{name}: double-click to rename, drag to reorder or onto the image to copy its layers",
  "tabs.rename": "Document name",

  "welcome.title": "Open an image or create a document",
  "welcome.open": "Open…",
  "welcome.new": "New document",
  "welcome.drop": "…or drop files here",

  "drop.newTab": "Drop to open in a new tab",
  "drop.layer": "Drop to add as a layer",
  "drop.copyLayers": "Drop to copy {name} as layers",
  "open.opening": "Opening {name}…",
  "open.failed": "Cannot open {name}: {error}",
  "open.warning.iccProfileUnsupported":
    "Embedded color profile not supported yet (LUT-based): colors read as sRGB",
  "open.warning.iccCurveApproximated": "Color profile tone curve approximated",
  "open.warning.firstFrameOnly": "Animated image: only the first frame was opened",
  "open.warning.firstPageOnly": "Several pages: only the first was opened",
  "open.warning.precisionReduced": "64-bit float samples stored as 32-bit floats",
  "open.warning.nonFiniteSamples":
    "Infinite or undefined (NaN) values: shown as the brightest value or as 0",
  "open.warning.colorInfoUnsupported":
    "The file's color information is not supported yet: colors read as sRGB",

  "open.error.io": "cannot read the file ({detail})",
  "open.error.decode": "damaged file or invalid image ({detail})",
  "open.error.notYetSupported": "the {detail} format is not supported yet",
  "open.error.heic": "HEIC/HEIF is not supported (HEVC patents)",
  "open.error.unsupportedPixels": "this kind of pixels is not supported yet ({detail})",
  "open.error.tooLarge": "the image is too large to open ({detail})",
  "open.error.unrecognized": "unrecognized image format",
  "open.error.internal": "internal error ({detail})",

  "export.hint": "Export ({mod}+Shift+E)",
  "export.title": "Export",
  "export.titleFor": "Export as {format}",
  "export.unsupportedExtension": "{name}: choose a PNG, TIFF or OpenEXR file",
  "export.format.png": "PNG",
  "export.format.tiff": "TIFF",
  "export.format.exr": "OpenEXR",
  "export.depth": "Bit depth",
  "export.depth.u8": "8-bit",
  "export.depth.u16": "16-bit",
  "export.depth.f16": "16-bit float (smaller, less precise)",
  "export.depth.f32": "32-bit float",
  "export.colorSpace": "Color space",
  "export.colorSpace.custom": "Same as source (embedded profile)",
  "export.compression": "Compression",
  "export.compression.fast": "Fast",
  "export.compression.small": "Smaller file (slower)",
  "export.compression.none": "None",
  "export.compression.deflate": "Deflate",
  "export.compression.lzw": "LZW (compatibility)",
  "export.alpha": "Keep transparency",
  "export.alpha.hint": "Without it, transparent areas are written over black",
  "export.dither": "Dither (reduces banding)",
  "export.confirm": "Export…",
  "export.cancel": "Cancel",
  "export.loadFailed": "Cannot read the export settings: {error}",
  "export.progress": "Exporting {name}… {percent}%",
  "export.stop": "Cancel export",
  "export.finished": "Exported {name}",
  "export.cancelled": "Export of {name} cancelled",
  "export.failed": "Cannot export {name}: {error}",
  "export.dismiss": "Dismiss",
  "export.report.clippedHigh": "{count} values above the format's range were clipped",
  "export.report.clippedLow": "{count} negative or out-of-gamut values were clipped",
  "export.report.nonFinite": "{count} infinite or NaN values were replaced",
  "export.report.halfOverflow": "{count} values exceeded the 16-bit float range",
  "export.report.precisionReduced": "Written as 16-bit floats: less precise than the image",
  "export.report.bigTiff": "Written as BigTIFF (over 4 GB): some older software cannot read it",
  "export.error.io": "cannot write the file ({detail})",
  "export.error.source": "rendering failed ({detail})",
  "export.error.cancelled": "cancelled",
  "export.error.unsupportedSpace": "this format cannot store this color space ({detail})",
  "export.error.tooLarge": "the image is too large for this format ({detail})",
  "export.error.invalidSpec": "invalid export settings ({detail})",
  "export.error.encode": "the encoder failed ({detail})",
  "export.error.documentClosed": "the document was closed",
  "export.error.internal": "internal error ({detail})",

  "document.info": "{width} × {height} px · {space}",

  "colorSpace.srgb": "sRGB",
  "colorSpace.linear-srgb": "Linear sRGB",
  "colorSpace.display-p3": "Display P3",
  "colorSpace.adobe-rgb": "Adobe RGB",
  "colorSpace.prophoto": "ProPhoto RGB",
  "colorSpace.rec2020": "Rec.2020",
  "colorSpace.linear-rec2020": "Linear Rec.2020",
  "colorSpace.rec2100-pq": "Rec.2100 PQ",
  "colorSpace.rec2100-hlg": "Rec.2100 HLG",
  "colorSpace.custom": "Custom",

  "layers.title": "Layers",
  "layers.empty": "No layers",
  "layers.show": "Show layer",
  "layers.hide": "Hide layer",
  "layers.renameHint": "Double-click or F2 to rename",
  "layers.opacity": "Opacity",
  "layers.delete": "Delete layer",
  "layers.fillColor": "Fill color",
  "layers.addFill": "Add fill layer",
  "layers.defaultFillName": "Fill {n}",

  "viewport.renderFailed": "Render failed: {error}",

  "view.zoom": "Zoom",
  "view.hint":
    "Wheel or pinch: zoom · Middle button or Space+drag: pan · {mod}+0: fit · {mod}+1: 100% · {mod}+/-: zoom steps",

  "status.gpu": "GPU: {name} ({backend})",
  "status.gpuInit": "Initializing GPU…",
  "status.gpuUnavailable": "GPU unavailable: {error}",
  "status.frameTime": "render {render} ms · frame {total} ms",
  "status.revision": "rev. {revision}",
} satisfies Record<string, string>;

export default en;
export type MessageKey = keyof typeof en;
export type Messages = Record<MessageKey, string>;
