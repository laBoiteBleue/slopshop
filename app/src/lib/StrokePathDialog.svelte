<script lang="ts" module>
  /** The tools Stroke Path paints with (Photoshop lists more; these two are SlopShop's). */
  export type StrokePathTool = "brush" | "eraser";

  /** The tool, and whether the pressure is simulated (from nothing to full and back). */
  export type StrokePathSettings = { tool: StrokePathTool; simulatePressure: boolean };
</script>

<script lang="ts">
  // The Paths panel's Stroke Path (ADR 0041), as Photoshop's dialog: the tool that paints along
  // the path, with its options of the moment, and Simulate Pressure. Enter applies, Esc cancels.
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let {
    settings: initial,
    onapply,
    onclose,
  }: {
    /** The settings used last. */
    settings: StrokePathSettings;
    onapply: (settings: StrokePathSettings) => void;
    onclose: () => void;
  } = $props();

  let tool = $state(untrack(() => initial.tool));
  let simulatePressure = $state(untrack(() => initial.simulatePressure));
  let dialog: HTMLDialogElement;

  function submit(e: SubmitEvent) {
    e.preventDefault();
    onapply({ tool, simulatePressure });
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
  aria-labelledby="stroke-path-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="stroke-path-title" {@attach movable("stroke-path")}>
      {t("paths.strokeDialog.title")}
    </header>
    <div class="fields">
      <label for="stroke-path-tool">{t("paths.strokeDialog.tool")}</label>
      <select id="stroke-path-tool" bind:value={tool}>
        <option value="brush">{t("tools.brush")}</option>
        <option value="eraser">{t("tools.eraser")}</option>
      </select>
      <label class="check">
        <input type="checkbox" bind:checked={simulatePressure} />
        {t("paths.strokeDialog.simulatePressure")}
      </label>
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary">{t("sizeDialog.ok")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 300px;
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
  }

  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 6px 10px;
    padding: 12px 10px;
  }

  .fields > label:not(.check) {
    color: var(--text-muted);
  }

  .check {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
