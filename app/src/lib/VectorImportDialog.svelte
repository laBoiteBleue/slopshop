<script lang="ts">
  // File > Open of a PDF or an SVG, as in Photoshop's Import PDF and Rasterize SVG: the pages as
  // thumbnails to pick (the first one by default; Shift+click picks a range; a single page is just
  // previewed), and the resolution they are rasterized at, or the size in pixels of the first
  // picked page (the resolution follows). Each page opens like a file: a tab, or a layer. The
  // resolution is remembered for the next file of the same kind.
  import { onMount, untrack } from "svelte";
  import { engine, type PageSize, type VectorInfo } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let {
    path,
    name,
    info,
    onopen,
    onclose,
  }: {
    path: string;
    /** The file's name, shown in the title. */
    name: string;
    info: VectorInfo;
    /** Open `pages` (from 0, in page order) at `dpi`. */
    onopen: (pages: number[], dpi: number) => void;
    onclose: () => void;
  } = $props();

  const MIN_DPI = 1;
  const MAX_DPI = 10_000;
  /** Thumbnail box side, in CSS pixels (a single page is previewed larger). */
  const THUMBNAIL = 104;
  const PREVIEW = 300;

  const { kind, defaultDpi, pages: sizes } = untrack(() => info);
  /** The PDF renderer's largest side, in pixels; SVGs are only bounded by memory. */
  const MAX_SIDE = kind === "pdf" ? 65_535 : 300_000;
  const STORAGE_KEY = `slopshop.${kind}Import.dpi`;
  const single = sizes.length === 1;
  let selected = $state<boolean[]>(sizes.map((_, i) => i === 0));
  let dpi = $state(savedDpi());
  /** The last page clicked, for Shift+click ranges. */
  let anchor = 0;
  let dialog: HTMLDialogElement;

  function savedDpi(): number {
    try {
      const saved = Number(localStorage.getItem(STORAGE_KEY));
      if (Number.isFinite(saved) && saved >= MIN_DPI && saved <= MAX_DPI) return saved;
    } catch {
      // Storage unavailable: the default.
    }
    return defaultDpi;
  }

  /** A page's size in pixels at `dpi`, rounded as the engine renders it. */
  function pixels(page: PageSize, dpi: number): [number, number] {
    return [Math.round((page.width * dpi) / 72), Math.round((page.height * dpi) / 72)];
  }

  const picked = $derived(selected.flatMap((on, i) => (on ? [i] : [])));
  const reference = $derived(picked[0] ?? 0);
  const [refWidth, refHeight] = $derived(pixels(sizes[reference], dpi));
  const dpiValid = $derived(Number.isFinite(dpi) && dpi >= MIN_DPI && dpi <= MAX_DPI);
  const tooLarge = $derived(
    dpiValid &&
      picked.some((i) => {
        const [w, h] = pixels(sizes[i], dpi);
        return w > MAX_SIDE || h > MAX_SIDE || w < 1 || h < 1;
      }),
  );
  const valid = $derived(picked.length > 0 && dpiValid && !tooLarge);

  /** Rounded for display; the exact value is kept so that typed pixel sizes come back. */
  const shownDpi = $derived(Math.round(dpi * 100) / 100);

  function setDpi(value: number) {
    if (Number.isFinite(value) && value > 0) dpi = value;
  }

  function onWidth(e: Event) {
    const px = (e.currentTarget as HTMLInputElement).valueAsNumber;
    setDpi((px * 72) / sizes[reference].width);
  }

  function onHeight(e: Event) {
    const px = (e.currentTarget as HTMLInputElement).valueAsNumber;
    setDpi((px * 72) / sizes[reference].height);
  }

  function toggle(index: number, e: MouseEvent) {
    if (e.shiftKey) {
      const [from, to] = index < anchor ? [index, anchor] : [anchor, index];
      selected = selected.map((on, i) => on || (i >= from && i <= to));
    } else {
      selected[index] = !selected[index];
    }
    anchor = index;
  }

  function selectAll(on: boolean) {
    selected = sizes.map(() => on);
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!valid) return;
    try {
      localStorage.setItem(STORAGE_KEY, String(dpi));
    } catch {
      // Not remembered; the import still happens.
    }
    onopen($state.snapshot(picked), dpi);
  }

  /** Renders the page's thumbnail once it scrolls into view. */
  function thumbnail(canvas: HTMLCanvasElement, page: number) {
    const size = sizes[page];
    const box = single ? PREVIEW : THUMBNAIL;
    const scale = box / Math.max(size.width, size.height);
    // The page's proportions before the pixels arrive.
    canvas.width = Math.max(1, Math.round(size.width * scale));
    canvas.height = Math.max(1, Math.round(size.height * scale));
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        observer.disconnect();
        const side = Math.round(box * devicePixelRatio);
        engine
          .vectorThumbnail(path, page, side)
          .then((image) => {
            canvas.width = image.width;
            canvas.height = image.height;
            canvas.getContext("2d")?.putImageData(image, 0, 0);
          })
          .catch(() => {
            // The page stays blank; opening it reports the problem.
          });
      },
      { rootMargin: "200px" },
    );
    observer.observe(canvas);
    return { destroy: () => observer.disconnect() };
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="vector-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="vector-title" {@attach movable("vector")}>
      {t(kind === "pdf" ? "vectorDialog.titlePdf" : "vectorDialog.titleSvg")} — {name}
    </header>
    <div class="body">
      <section class="pages" aria-label={t("vectorDialog.pages")}>
        {#if single}
          <div class="preview"><canvas use:thumbnail={0}></canvas></div>
        {:else}
          <div class="grid">
            {#each sizes as _, i (i)}
              <button
                type="button"
                class="page"
                class:selected={selected[i]}
                aria-pressed={selected[i]}
                aria-label={t("vectorDialog.page", { number: i + 1 })}
                onclick={(e) => toggle(i, e)}
              >
                <span class="sheet"><canvas use:thumbnail={i}></canvas></span>
                <span class="number">{i + 1}</span>
              </button>
            {/each}
          </div>
          <div class="selection">
            <span>{t("vectorDialog.selected", { count: picked.length, total: sizes.length })}</span>
            <button type="button" class="btn small" onclick={() => selectAll(true)}>
              {t("vectorDialog.all")}
            </button>
            <button type="button" class="btn small" onclick={() => selectAll(false)}>
              {t("vectorDialog.none")}
            </button>
          </div>
        {/if}
      </section>
      <section class="fields">
        <label for="vector-dpi">{t("vectorDialog.resolution")}</label>
        <span class="unit-field">
          <input
            id="vector-dpi"
            type="number"
            step="any"
            min={MIN_DPI}
            max={MAX_DPI}
            value={shownDpi}
            oninput={(e) => setDpi((e.currentTarget as HTMLInputElement).valueAsNumber)}
          />
          {t("vectorDialog.ppi")}
        </span>
        <span class="caption">
          {single ? t("vectorDialog.size") : t("vectorDialog.sizeOf", { number: reference + 1 })}
        </span>
        <label for="vector-width">{t("vectorDialog.width")}</label>
        <span class="unit-field">
          <input id="vector-width" type="number" min="1" value={refWidth} oninput={onWidth} />
          {t("vectorDialog.px")}
        </span>
        <label for="vector-height">{t("vectorDialog.height")}</label>
        <span class="unit-field">
          <input id="vector-height" type="number" min="1" value={refHeight} oninput={onHeight} />
          {t("vectorDialog.px")}
        </span>
        <span class="label">{t("vectorDialog.mode")}</span>
        <span class="mode">{t("vectorDialog.modeValue")}</span>
        {#if picked.length === 0}
          <p class="invalid">{t("vectorDialog.noPage")}</p>
        {:else if tooLarge}
          <p class="invalid">{t("vectorDialog.tooLarge", { max: MAX_SIDE })}</p>
        {/if}
      </section>
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("vectorDialog.cancel")}</button>
      <button type="submit" class="btn primary" disabled={!valid}>{t("vectorDialog.ok")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 640px;
    max-width: calc(100vw - 32px);
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  dialog::backdrop {
    background: #00000055;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .body {
    display: grid;
    grid-template-columns: 1fr 230px;
    gap: 10px;
    padding: 10px;
  }

  .pages {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(116px, 1fr));
    gap: 6px;
    height: 360px;
    overflow-y: auto;
    padding: 4px;
    background: var(--pasteboard);
    border: 1px solid var(--border-dark);
    align-content: start;
  }

  .preview {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 360px;
    background: var(--pasteboard);
    border: 1px solid var(--border-dark);
  }

  .preview canvas {
    max-width: 100%;
    max-height: 340px;
    /* A checkerboard behind transparent drawings. */
    background: repeating-conic-gradient(#ccc 0% 25%, #fff 0% 50%) 0 0 / 16px 16px;
    box-shadow: 0 1px 3px #0008;
  }

  .page {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 4px;
    border: 2px solid transparent;
    border-radius: 3px;
    background: transparent;
  }

  .page:hover {
    background: var(--hover);
  }

  .page.selected {
    border-color: var(--accent);
  }

  .sheet {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 104px;
    height: 104px;
  }

  .sheet canvas {
    max-width: 104px;
    max-height: 104px;
    background: #fff;
    box-shadow: 0 1px 3px #0008;
  }

  .number {
    color: var(--text-muted);
  }

  .page.selected .number {
    color: var(--text);
  }

  .selection {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }

  .selection span {
    flex: 1;
  }

  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    align-content: start;
    gap: 6px 10px;
  }

  .fields > label,
  .label {
    color: var(--text-muted);
  }

  .caption {
    grid-column: 1 / -1;
    margin-top: 6px;
    color: var(--text-muted);
  }

  .unit-field {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }

  .unit-field input {
    width: 80px;
    min-width: 0;
  }

  .mode {
    color: var(--text);
  }

  .invalid {
    grid-column: 1 / -1;
    margin: 0;
    color: var(--danger-fg);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
