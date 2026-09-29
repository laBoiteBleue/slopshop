// English catalog: the reference. Every other catalog must provide exactly these keys
// (enforced by the `Messages` type). Placeholders use `{name}`.
const en = {
  "app.preAlpha": "pre-alpha",
  "app.loading": "Loading document…",
  "app.language": "Language",

  "toolbar.undoHint": "Undo (Ctrl+Z)",
  "toolbar.redoHint": "Redo (Ctrl+Shift+Z)",

  "document.untitled": "Untitled",
  "document.info": "{width} × {height} px · {space}",

  "colorSpace.linear-srgb": "Linear sRGB",
  "colorSpace.srgb": "sRGB",

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

  "status.gpu": "GPU: {name} ({backend})",
  "status.gpuInit": "Initializing GPU…",
  "status.gpuUnavailable": "GPU unavailable: {error}",
  "status.frameTime": "frame {ms} ms",
  "status.revision": "rev. {revision}",
} satisfies Record<string, string>;

export default en;
export type MessageKey = keyof typeof en;
export type Messages = Record<MessageKey, string>;
