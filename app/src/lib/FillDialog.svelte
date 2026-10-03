<script lang="ts" module>
  import type { MessageKey } from "./i18n/en";

  /** What Edit > Fill paints: a color (`color`: chosen in the color picker next). */
  export type FillContents = "foreground" | "background" | "color" | "black" | "gray" | "white";

  /** The contents and the opacity, in `[0, 1]`. */
  export type FillSettings = { contents: FillContents; opacity: number };

  const CONTENTS: { value: FillContents; label: MessageKey }[] = [
    { value: "foreground", label: "fillChoice.foreground" },
    { value: "background", label: "fillChoice.background" },
    { value: "color", label: "fillChoice.color" },
    { value: "black", label: "fillChoice.black" },
    { value: "gray", label: "fillChoice.gray" },
    { value: "white", label: "fillChoice.white" },
  ];

  /** As Photoshop's Fill dialog, the last settings come back (for the session). */
  let last: FillSettings = { contents: "foreground", opacity: 1 };
</script>

<script lang="ts">
  // Edit > Fill (Shift+F5), laid out as Photoshop's Fill dialog (Contents, then Opacity; OK and
  // Cancel on the right): the selection of the active layer, or the whole layer without one,
  // painted with a color (paint, ADR 0027 and 0029). Enter applies, Esc cancels.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import SliderField from "./SliderField.svelte";

  let {
    onchoose,
    onclose,
  }: {
    onchoose: (settings: FillSettings) => void;
    onclose: () => void;
  } = $props();

  let contents = $state(last.contents);
  let opacity = $state(last.opacity);
  let dialog: HTMLDialogElement;
  let form: HTMLFormElement;

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog. Enter on the closed list
    // applies, as in Photoshop (a select does not submit its form by itself).
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
    last = { contents, opacity };
    onchoose(last);
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="fill-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="fill-title">{t("fillChoice.title")}</header>
  <form bind:this={form} onsubmit={submit}>
    <div class="fields">
      <label for="fill-contents">{t("fillChoice.contents")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <select id="fill-contents" bind:value={contents} autofocus>
        {#each CONTENTS as entry (entry.value)}
          <option value={entry.value}>{t(entry.label)}</option>
        {/each}
      </select>
      <div class="row">
        <SliderField
          label={t("fillChoice.opacity")}
          bind:value={opacity}
          min={0}
          max={100}
          unit="%"
          factor={100}
          width={52}
        />
      </div>
    </div>
    <div class="buttons">
      <button type="submit" class="btn primary">{t("sizeDialog.ok")}</button>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 360px;
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

  /* Photoshop's layout: the settings on the left, OK and Cancel stacked on the right. */
  form {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 14px;
    padding: 14px;
  }

  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    align-self: start;
    gap: 8px;
  }

  label {
    color: var(--text-muted);
  }

  /* The slider field brings its own (scrubby) label. */
  .row {
    grid-column: 1 / -1;
  }

  select {
    min-width: 0;
  }

  .buttons {
    display: grid;
    align-content: start;
    gap: 6px;
  }

  .buttons .btn {
    min-width: 80px;
  }
</style>
