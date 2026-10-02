<script lang="ts">
  // Delete with a selection (ADR 0027): what becomes of the selected pixels of the active layer,
  // as Photoshop's Fill dialog asks. Erase lowers their alpha only; the fills paint the
  // foreground or the background color. Either is paint, which Layer > Delete Paint removes.
  // 1, 2 and 3 pick a choice from the keyboard; Enter takes the focused one, Esc cancels.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";

  type Choice = "erase" | "foreground" | "background";

  let {
    foreground,
    background,
    onchoose,
    onclose,
  }: {
    /** The colors, `#rrggbb` sRGB. */
    foreground: string;
    background: string;
    onchoose: (choice: Choice) => void;
    onclose: () => void;
  } = $props();

  const CHOICES: { choice: Choice; label: MessageKey }[] = [
    { choice: "erase", label: "fillChoice.erase" },
    { choice: "foreground", label: "fillChoice.foreground" },
    { choice: "background", label: "fillChoice.background" },
  ];

  let dialog: HTMLDialogElement;

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog; 1–3 choose.
    const keys = (e: KeyboardEvent) => {
      e.stopPropagation();
      const index = ["1", "2", "3"].indexOf(e.key);
      if (index >= 0 && !e.repeat) {
        e.preventDefault();
        onchoose(CHOICES[index].choice);
      }
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="fill-choice-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="fill-choice-title">{t("fillChoice.title")}</header>
  <div class="choices">
    {#each CHOICES as entry, index (entry.choice)}
      <!-- svelte-ignore a11y_autofocus -->
      <button class="choice" autofocus={index === 0} onclick={() => onchoose(entry.choice)}>
        <span
          class="swatch"
          class:transparent={entry.choice === "erase"}
          style:background={entry.choice === "foreground"
            ? foreground
            : entry.choice === "background"
              ? background
              : undefined}
        ></span>
        <span class="label">{t(entry.label)}</span>
        <span class="key">{index + 1}</span>
      </button>
    {/each}
  </div>
  <footer>
    <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
  </footer>
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

  .choices {
    display: grid;
    gap: 4px;
    padding: 10px;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
    background: var(--chrome);
    color: var(--text);
    text-align: left;
  }

  .choice:hover,
  .choice:focus-visible {
    background: var(--selected);
    outline: none;
  }

  .swatch {
    flex: none;
    width: 18px;
    height: 18px;
    border: 1px solid var(--border-dark);
  }

  /* Transparency, as the canvas shows it. */
  .swatch.transparent {
    background: repeating-conic-gradient(#bbb 0 25%, #fff 0 50%) 0 0 / 8px 8px;
  }

  .label {
    flex: 1;
  }

  .key {
    color: var(--text-muted);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
