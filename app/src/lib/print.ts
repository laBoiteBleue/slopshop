/**
 * File > Print: `jpeg` (the page the engine rendered) through the system's print dialog, from
 * a hidden frame holding only the image, fitted to the paper. Settles once the dialog is done.
 */
export async function printImage(
  jpeg: ArrayBuffer,
  title: string,
  landscape: boolean,
): Promise<void> {
  const url = URL.createObjectURL(new Blob([jpeg], { type: "image/jpeg" }));
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
    const style = page.createElement("style");
    // No page margins, so that the browser prints no header or footer (date, title, address);
    // the image keeps a margin of its own, centered and whole on the page, the page turned to
    // the image's orientation.
    style.textContent = `
      @page { margin: 0; size: ${landscape ? "landscape" : "portrait"}; }
      html, body { margin: 0; height: 100%; }
      body {
        box-sizing: border-box;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 10mm;
      }
      img { max-width: 100%; max-height: 100%; object-fit: contain; }
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
    URL.revokeObjectURL(url);
  }
}
