// File > Print: where the image goes on the paper (Photoshop's Print Settings), and the
// printing itself, from a hidden frame through the system's print dialog. Lengths in mm.

/** Paper sizes, portrait (width × height, mm). */
export const PAPERS = [
  { id: "a4", width: 210, height: 297 },
  { id: "a3", width: 297, height: 420 },
  { id: "a5", width: 148, height: 210 },
  { id: "letter", width: 215.9, height: 279.4 },
  { id: "legal", width: 215.9, height: 355.6 },
  { id: "photo10x15", width: 101.6, height: 152.4 },
  { id: "photo13x18", width: 127, height: 177.8 },
  { id: "photo20x25", width: 203.2, height: 254 },
] as const;

export type PaperId = (typeof PAPERS)[number]["id"];

/** The margin Scale to Fit leaves around the image, mm (most printers cannot print closer). */
export const FIT_MARGIN = 10;

export type PrintSettings = {
  paper: PaperId;
  landscape: boolean;
  /** Scale to fit the paper (within its margin); else at `ppi`. */
  fit: boolean;
  /** Image pixels per inch when not fitted. */
  ppi: number;
  /** Centered on the paper; else at `left`, `top` (mm from the paper's corner). */
  center: boolean;
  left: number;
  top: number;
};

/** The paper and the image on it, mm. */
export type PrintLayout = {
  pageWidth: number;
  pageHeight: number;
  left: number;
  top: number;
  width: number;
  height: number;
  /** Image pixels per inch on the paper. */
  ppi: number;
};

/** Where an image of `width` × `height` pixels lands on the paper, as `settings` say. */
export function layoutOf(settings: PrintSettings, width: number, height: number): PrintLayout {
  const paper = PAPERS.find((p) => p.id === settings.paper) ?? PAPERS[0];
  const [pageWidth, pageHeight] = settings.landscape
    ? [paper.height, paper.width]
    : [paper.width, paper.height];
  // Millimeters per image pixel.
  const mm = settings.fit
    ? Math.min((pageWidth - 2 * FIT_MARGIN) / width, (pageHeight - 2 * FIT_MARGIN) / height)
    : 25.4 / settings.ppi;
  const [w, h] = [width * mm, height * mm];
  const centered = settings.center || settings.fit;
  return {
    pageWidth,
    pageHeight,
    left: centered ? (pageWidth - w) / 2 : settings.left,
    top: centered ? (pageHeight - h) / 2 : settings.top,
    width: w,
    height: h,
    ppi: 25.4 / mm,
  };
}

/**
 * Print the image at `url` laid out as `layout`, through the system's print dialog (the
 * webview's own preview is turned off: see `additionalBrowserArgs`), from a hidden frame
 * holding only the image. Settles once the dialog is done.
 */
export async function printImage(url: string, layout: PrintLayout, title: string): Promise<void> {
  const frame = document.createElement("iframe");
  frame.setAttribute("aria-hidden", "true");
  frame.tabIndex = -1;
  Object.assign(frame.style, {
    position: "fixed",
    right: "0",
    bottom: "0",
    width: "0",
    height: "0",
    border: "0",
  });
  document.body.append(frame);
  try {
    const page = frame.contentDocument;
    const view = frame.contentWindow;
    if (!page || !view) throw new Error("no print frame");
    page.title = title;
    const mm = (v: number) => `${v.toFixed(2)}mm`;
    const style = page.createElement("style");
    // No page margins: the browser prints no header or footer, and the image goes exactly
    // where the layout says (parts beyond the paper are cut).
    style.textContent = `
      @page { margin: 0; size: ${mm(layout.pageWidth)} ${mm(layout.pageHeight)}; }
      html, body { margin: 0; }
      body { position: relative; width: ${mm(layout.pageWidth)}; height: ${mm(layout.pageHeight)};
        overflow: hidden; }
      img { position: absolute; left: ${mm(layout.left)}; top: ${mm(layout.top)};
        width: ${mm(layout.width)}; height: ${mm(layout.height)}; }
    `;
    page.head.append(style);
    const image = page.createElement("img");
    image.src = url;
    page.body.append(image);
    await image.decode();
    const done = new Promise<void>((resolve) => {
      view.addEventListener("afterprint", () => resolve(), { once: true });
    });
    view.focus();
    view.print();
    await done;
  } finally {
    frame.remove();
  }
}
