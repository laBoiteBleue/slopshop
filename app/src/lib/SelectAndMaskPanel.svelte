<script lang="ts" module>
  import type { EdgeSettings, SelectionViewMode } from "./engine";

  /** Where Select and Mask's result goes. */
  export type RefineOutput = "selection" | "layerMask" | "newLayer";

  /** Select and Mask's settings, kept from one use to the next. */
  export type RefineSettings = {
    view: SelectionViewMode;
    /** Edge detection: ViTMatte's band around the outline, document pixels. */
    radius: number;
    edges: EdgeSettings;
    /** The refine-edge brush: on (strokes on the image), its size, and whether it erases. */
    brush: { on: boolean; size: number; erase: boolean };
    output: RefineOutput;
  };

  export const DEFAULT_REFINE: RefineSettings = {
    view: "overlay",
    radius: 16,
    edges: { smooth: 0, feather: 0, contrast: 0, shift: 0 },
    brush: { on: false, size: 40, erase: false },
    output: "selection",
  };
</script>

<script lang="ts">
  // Select > Select and Mask, as a light panel beside the image (maintainer's choice,
  // 2026-10-04), not Photoshop's workspace: how the selection is shown, edge detection
  // (ViTMatte, run on request: it takes seconds; the refine-edge brush marks where else it
  // decides, hair and fur, and runs it on release), then Smooth, Shift Edge, Feather and
  // Contrast shown live on the image, and where the result goes. Enter applies (except in a
  // number field), Esc cancels.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";
  import SliderField from "./SliderField.svelte";
  import { MAX_BRUSH, MAX_FEATHER, MAX_REFINE } from "./selection";
  import { keepFocus } from "./platform";

  let {
    settings = $bindable(),
    canOutputToLayer,
    busy = false,
    onview,
    onedges,
    ondetect,
    onapply,
    onclose,
  }: {
    settings: RefineSettings;
    /** An active layer can take the result as a mask. */
    canOutputToLayer: boolean;
    /** Edge detection runs. */
    busy?: boolean;
    onview: (view: SelectionViewMode) => void;
    /** The edge settings changed: shown live. */
    onedges: (edges: EdgeSettings) => void;
    ondetect: (radius: number) => void;
    onapply: () => void;
    onclose: () => void;
  } = $props();

  const VIEWS: [SelectionViewMode, MessageKey][] = [
    ["ants", "refine.view.ants"],
    ["overlay", "refine.view.overlay"],
    ["onBlack", "refine.view.onBlack"],
    ["onWhite", "refine.view.onWhite"],
    ["mask", "refine.view.mask"],
  ];
  const OUTPUTS: [RefineOutput, MessageKey][] = [
    ["selection", "refine.output.selection"],
    ["layerMask", "refine.output.layerMask"],
    ["newLayer", "refine.output.newLayer"],
  ];

  $effect(() => onview(settings.view));
  $effect(() => {
    const { smooth, feather, contrast, shift } = settings.edges;
    onedges({ smooth, feather, contrast, shift });
  });

  onMount(() => {
    const keys = (e: KeyboardEvent) => {
      const field = e.target instanceof HTMLInputElement && e.target.type === "number";
      if (e.key === "Enter" && !field && !busy) onapply();
      else if (e.key === "Escape") onclose();
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });
</script>

<section class="panel" aria-labelledby="refine-title">
  <header id="refine-title">{t("refine.title")}</header>
  <div class="body">
    <label class="row">
      <span>{t("refine.view")}</span>
      <select bind:value={settings.view}>
        {#each VIEWS as [view, label] (view)}
          <option value={view}>{t(label)}</option>
        {/each}
      </select>
    </label>

    <h3>{t("refine.detection")}</h3>
    <div class="detect">
      <SliderField
        label={t("refine.radius")}
        bind:value={settings.radius}
        min={1}
        max={MAX_REFINE}
        unit="px"
        log
      />
      <button
        type="button"
        class="btn small"
        disabled={busy}
        onclick={() => ondetect(settings.radius)}
      >
        {t("refine.detect")}
      </button>
    </div>

    <div class="brush">
      <button
        type="button"
        class="btn small"
        class:on={settings.brush.on}
        aria-pressed={settings.brush.on}
        title={t("refine.brush.hint")}
        onmousedown={keepFocus}
        onclick={() => (settings.brush.on = !settings.brush.on)}
      >
        {t("refine.brush")}
      </button>
      {#if settings.brush.on}
        <div class="segmented" role="group" aria-label={t("refine.brush")}>
          {#each [false, true] as erase (erase)}
            <button
              type="button"
              class:on={settings.brush.erase === erase}
              aria-pressed={settings.brush.erase === erase}
              onmousedown={keepFocus}
              onclick={() => (settings.brush.erase = erase)}
            >
              {t(erase ? "refine.brush.erase" : "refine.brush.paint")}
            </button>
          {/each}
        </div>
      {/if}
    </div>
    {#if settings.brush.on}
      <SliderField
        label={t("refine.brush.size")}
        bind:value={settings.brush.size}
        min={1}
        max={MAX_BRUSH}
        unit="px"
        log
      />
    {/if}

    <h3>{t("refine.edges")}</h3>
    <div class="fields">
      <SliderField
        label={t("refine.smooth")}
        bind:value={settings.edges.smooth}
        min={0}
        max={100}
        unit="px"
      />
      <SliderField
        label={t("refine.feather")}
        bind:value={settings.edges.feather}
        min={0}
        max={MAX_FEATHER}
        step={0.5}
        unit="px"
      />
      <SliderField
        label={t("refine.contrast")}
        bind:value={settings.edges.contrast}
        min={0}
        max={100}
        unit="%"
      />
      <SliderField
        label={t("refine.shift")}
        bind:value={settings.edges.shift}
        min={-100}
        max={100}
        unit="px"
      />
    </div>

    <label class="row">
      <span>{t("refine.output")}</span>
      <select bind:value={settings.output}>
        {#each OUTPUTS as [output, label] (output)}
          <option value={output} disabled={output !== "selection" && !canOutputToLayer}>
            {t(label)}
          </option>
        {/each}
      </select>
    </label>
  </div>
  <footer>
    <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
    <button type="button" class="btn primary" disabled={busy} onclick={onapply}>
      {t("sizeDialog.ok")}
    </button>
  </footer>
</section>

<style>
  .panel {
    position: fixed;
    top: 96px;
    right: 276px;
    z-index: 15;
    display: grid;
    width: 280px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    box-shadow: 0 10px 32px #0009;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .body {
    display: grid;
    gap: 8px;
    padding: 10px;
  }

  h3 {
    margin: 4px 0 0;
    font-size: inherit;
    color: var(--text-muted);
  }

  .row {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
  }

  .detect {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }

  .fields {
    display: grid;
    gap: 6px;
  }

  .brush {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }

  .btn.on {
    background: var(--selected);
  }

  .segmented {
    display: inline-flex;
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    overflow: hidden;
  }

  .segmented button {
    padding: 2px 8px;
    border: none;
    background: transparent;
    color: var(--text-muted);
  }

  .segmented button + button {
    border-left: 1px solid var(--border-strong);
  }

  .segmented button.on {
    background: var(--accent);
    color: #ffffff;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
