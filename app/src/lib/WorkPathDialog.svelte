<script lang="ts">
  // The Paths panel's Make Work Path (ADR 0041), as Photoshop's dialog: the Tolerance, in
  // pixels (0.5 to 10), within which the selection's outline is followed; lower follows it
  // more closely with more anchors. Enter applies, Esc cancels.
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import { MAX_TOLERANCE, MIN_TOLERANCE } from "./paths";

  let {
    tolerance: initial,
    onapply,
    onclose,
  }: {
    /** The tolerance used last. */
    tolerance: number;
    onapply: (tolerance: number) => void;
    onclose: () => void;
  } = $props();

  let tolerance = $state(untrack(() => initial));
  let dialog: HTMLDialogElement;
  let field: HTMLInputElement;
  const valid = $derived(
    Number.isFinite(tolerance) && tolerance >= MIN_TOLERANCE && tolerance <= MAX_TOLERANCE,
  );

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(tolerance);
  }

  onMount(() => {
    dialog.showModal();
    field.select();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="work-path-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="work-path-title" {@attach movable("work-path")}>
      {t("paths.workPathDialog.title")}
    </header>
    <div class="fields">
      <label for="work-path-tolerance">{t("paths.workPathDialog.tolerance")}</label>
      <span class="value">
        <input
          id="work-path-tolerance"
          type="number"
          min={MIN_TOLERANCE}
          max={MAX_TOLERANCE}
          step="0.1"
          bind:this={field}
          bind:value={tolerance}
        />
        {t("units.px")}
      </span>
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary" disabled={!valid}>{t("sizeDialog.ok")}</button>
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

  .fields > label {
    color: var(--text-muted);
  }

  .value {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  input {
    width: 80px;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
