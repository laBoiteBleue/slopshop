<script lang="ts">
  // Image > Adjustments (ADR 0029), laid out as Photoshop's adjustment dialogs: the settings
  // on the left, OK, Cancel and Preview on the right. The app previews the settings on the
  // canvas while the dialog is open; OK applies them to the selected layers as an effect of
  // their stack (one undo entry), Cancel leaves everything as it was. Enter applies, Esc
  // cancels.
  import { onMount } from "svelte";
  import AdjustmentFields from "./AdjustmentFields.svelte";
  import type { LayerView } from "./engine";
  import { t } from "./i18n/index.svelte";

  let {
    adjustment,
    preview,
    onlive,
    oncurves,
    onpreview,
    onok,
    oncancel,
  }: {
    /** The adjustment with the settings chosen so far. */
    adjustment: NonNullable<LayerView["adjustment"]>;
    /** The canvas shows the settings. */
    preview: boolean;
    /** Settings changed (the canvas follows them). */
    onlive: (values: number[]) => void;
    /** Curves' points changed: composite, red, green, blue. */
    oncurves: (curves: number[][][]) => void;
    onpreview: (preview: boolean) => void;
    onok: () => void;
    oncancel: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let form: HTMLFormElement;

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog. Enter applies, as in
    // Photoshop, also from a list.
    const keys = (e: KeyboardEvent) => {
      e.stopPropagation();
      if (e.key === "Enter" && e.target instanceof HTMLSelectElement) {
        e.preventDefault();
        form.requestSubmit();
      }
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    onok();
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="adjust-title"
  oncancel={(e) => {
    e.preventDefault();
    oncancel();
  }}
>
  <header id="adjust-title">{t(`adjustment.${adjustment.id}`)}</header>
  <form bind:this={form} onsubmit={submit}>
    <div class="settings">
      <AdjustmentFields
        {adjustment}
        owner={0}
        {onlive}
        onapply={onlive}
        onend={() => {}}
        oncurveslive={oncurves}
        oncurvesapply={oncurves}
      />
    </div>
    <div class="buttons">
      <!-- svelte-ignore a11y_autofocus -->
      <button type="submit" class="btn primary" autofocus>{t("sizeDialog.ok")}</button>
      <button type="button" class="btn" onclick={oncancel}>{t("sizeDialog.cancel")}</button>
      <label class="check">
        <input
          type="checkbox"
          checked={preview}
          onchange={(e) => onpreview((e.currentTarget as HTMLInputElement).checked)}
        />
        {t("adjustDialog.preview")}
      </label>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 400px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  /* The canvas behind shows the preview: no darkening. */
  dialog::backdrop {
    background: transparent;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  /* Photoshop's layout: the settings on the left, OK and Cancel stacked on the right. */
  form {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 10px;
    padding: 8px 10px 10px 4px;
  }

  .settings {
    min-width: 0;
  }

  .buttons {
    display: grid;
    align-content: start;
    gap: 6px;
    padding-top: 4px;
  }

  .buttons .btn {
    min-width: 80px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }
</style>
