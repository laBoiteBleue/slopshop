<script lang="ts" module>
  import type { MessageKey } from "./i18n/en";

  /** What Delete does to the selection: erase it, paint a color, or fill it generatively. */
  export type ClearContents = "transparent" | "background" | "foreground" | "generative";

  const CONTENTS: { value: ClearContents; label: MessageKey }[] = [
    { value: "transparent", label: "clearChoice.transparent" },
    { value: "background", label: "clearChoice.background" },
    { value: "foreground", label: "clearChoice.foreground" },
    { value: "generative", label: "clearChoice.generative" },
  ];

  /** The last choice comes back (for the session), as the Fill dialog's does. */
  let last: ClearContents = "transparent";
</script>

<script lang="ts">
  // Delete with a selection on a pixel layer (ADR 0045): the selection made transparent, painted
  // with the background or the foreground color, or filled by the generative model with what
  // surrounds it. The choices are radio buttons (arrows move, Enter applies, Esc cancels); the
  // last one is preselected.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let {
    colors,
    generative,
    onchoose,
    onclose,
  }: {
    /** The foreground and background colors, `#rrggbb` sRGB, shown next to their choices. */
    colors: { foreground: string; background: string };
    /** Whether generative fill is offered on this machine. */
    generative: boolean;
    onchoose: (contents: ClearContents) => void;
    onclose: () => void;
  } = $props();

  /** The last choice, unless it is not offered. */
  function initial(): ClearContents {
    return last === "generative" && !generative ? "transparent" : last;
  }

  let contents = $state<ClearContents>(initial());
  let dialog: HTMLDialogElement;

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const keys = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    last = contents;
    onchoose(contents);
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="clear-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="clear-title" {@attach movable("clear")}>{t("clearChoice.title")}</header>
  <form onsubmit={submit}>
    <fieldset>
      <legend>{t("clearChoice.contents")}</legend>
      {#each CONTENTS as entry (entry.value)}
        {@const disabled = entry.value === "generative" && !generative}
        <label class:disabled>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            type="radio"
            name="clear-contents"
            value={entry.value}
            bind:group={contents}
            {disabled}
            autofocus={entry.value === contents}
          />
          <span>{t(entry.label)}</span>
          {#if entry.value === "background" || entry.value === "foreground"}
            <span class="swatch" style:background={colors[entry.value]}></span>
          {/if}
        </label>
      {/each}
      <p class="hint">
        {generative ? t("clearChoice.generativeHint") : t("clearChoice.generativeUnsupported")}
      </p>
    </fieldset>
    <div class="buttons">
      <button type="submit" class="btn primary">{t("sizeDialog.ok")}</button>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 380px;
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

  form {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 14px;
    padding: 14px;
  }

  fieldset {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 0;
    border: none;
  }

  legend {
    margin-bottom: 6px;
    color: var(--text-muted);
  }

  label {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  label.disabled {
    color: var(--text-muted);
  }

  .swatch {
    width: 28px;
    height: 14px;
    border: 1px solid var(--border-dark);
    border-radius: 2px;
  }

  .hint {
    margin: 4px 0 0;
    color: var(--text-muted);
    font-size: 0.92em;
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
