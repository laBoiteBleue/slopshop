<script lang="ts">
  // A pattern chosen from the library (ADR 0042): Layer > New Fill Layer > Pattern, and the
  // Properties panel's pattern. The generated patterns, then the user's (Edit > Define
  // Pattern); a click picks one.
  import { onMount } from "svelte";
  import { engine, type PatternEntry } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import { patternName } from "./patterns";
  import PatternThumbnail from "./PatternThumbnail.svelte";

  let {
    title,
    onpick,
    onclose,
  }: {
    title: string;
    /** The pattern chosen, and what the user knows it by. */
    onpick: (entry: PatternEntry, name: string) => void;
    onclose: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let entries = $state<PatternEntry[] | null>(null);
  let failed = $state(false);

  onMount(() => {
    dialog.showModal();
    engine
      .listPatterns()
      .then((list) => (entries = list))
      .catch(() => (failed = true));
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="pattern-picker-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="pattern-picker-title" {@attach movable("pattern-picker")}>{title}</header>
  <div class="grid" role="listbox" aria-label={t("patterns.library")}>
    {#if failed}
      <p class="note">{t("patterns.failed")}</p>
    {:else if entries}
      {#each entries as entry (entry.id)}
        {@const name = patternName(entry, t)}
        <button
          type="button"
          class="pattern"
          role="option"
          aria-selected="false"
          title={name}
          aria-label={name}
          onclick={() => onpick(entry, name)}
        >
          <PatternThumbnail pattern={entry.id} size={48} />
        </button>
      {/each}
    {/if}
  </div>
  <footer>
    <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: 340px;
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

  .grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    max-height: 320px;
    overflow-y: auto;
    padding: 10px;
  }

  .pattern {
    display: inline-flex;
    padding: 2px;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
    background: none;
    cursor: pointer;
  }

  .pattern:hover,
  .pattern:focus-visible {
    border-color: var(--accent);
  }

  .note {
    margin: 0;
    color: var(--text-muted);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
