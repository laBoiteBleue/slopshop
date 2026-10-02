<script lang="ts" module>
  import type { MessageKey } from "./i18n/en";

  /** What the new document's layer holds: a color, or nothing (a transparent layer). */
  export type NewBackground = "white" | "black" | "background" | "transparent";

  export type NewDocumentSettings = {
    /** `null`: untitled. */
    name: string | null;
    width: number;
    height: number;
    background: NewBackground;
  };

  /** Common sizes, in pixels (print sizes at 300 ppi). */
  const PRESETS: { id: string; label: MessageKey; width: number; height: number }[] = [
    { id: "photo", label: "newDocument.preset.photo", width: 6000, height: 4000 },
    { id: "a4", label: "newDocument.preset.a4", width: 2480, height: 3508 },
    { id: "letter", label: "newDocument.preset.letter", width: 2550, height: 3300 },
    { id: "hd", label: "newDocument.preset.hd", width: 1920, height: 1080 },
    { id: "uhd", label: "newDocument.preset.uhd", width: 3840, height: 2160 },
    { id: "square", label: "newDocument.preset.square", width: 1080, height: 1080 },
    { id: "story", label: "newDocument.preset.story", width: 1080, height: 1920 },
  ];

  const BACKGROUNDS: { value: NewBackground; label: MessageKey }[] = [
    { value: "white", label: "newDocument.background.white" },
    { value: "black", label: "newDocument.background.black" },
    { value: "background", label: "newDocument.background.color" },
    { value: "transparent", label: "newDocument.background.transparent" },
  ];

  /** Largest side, in pixels (as the engine accepts). */
  const MAX_SIDE = 300_000;

  /** As Photoshop's New dialog, the last settings come back (for the session). */
  let last = { width: 6000, height: 4000, background: "white" as NewBackground };
</script>

<script lang="ts">
  // File > New, as Photoshop's New dialog: a name, a preset, the size in pixels with its
  // orientation, and the background contents; OK and Cancel on the right. Enter creates the
  // document, Esc cancels.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import Icon from "./Icon.svelte";

  let {
    oncreate,
    onclose,
  }: {
    oncreate: (settings: NewDocumentSettings) => void;
    onclose: () => void;
  } = $props();

  let name = $state("");
  let width = $state(last.width);
  let height = $state(last.height);
  let background = $state(last.background);
  let dialog: HTMLDialogElement;
  let form: HTMLFormElement;

  const valid = $derived(
    [width, height].every((side) => Number.isInteger(side) && side >= 1 && side <= MAX_SIDE),
  );
  /** The preset matching the size, either way round; none is Custom. */
  const preset = $derived(
    PRESETS.find(
      (p) =>
        (p.width === width && p.height === height) || (p.width === height && p.height === width),
    )?.id ?? "custom",
  );
  const portrait = $derived(height > width);

  function choosePreset(id: string) {
    const chosen = PRESETS.find((p) => p.id === id);
    if (chosen) [width, height] = [chosen.width, chosen.height];
  }

  /** Portrait or landscape: the sides swap if needed. */
  function orient(toPortrait: boolean) {
    if (toPortrait !== portrait && width !== height) [width, height] = [height, width];
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!valid) return;
    last = { width, height, background };
    const trimmed = name.trim();
    oncreate({ name: trimmed === "" ? null : trimmed, width, height, background });
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog. Enter on a closed list
    // creates the document, as in the fields (a select does not submit its form by itself).
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
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="new-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="new-title">{t("newDocument.title")}</header>
  <form bind:this={form} onsubmit={submit}>
    <div class="fields">
      <label for="new-name">{t("newDocument.name")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="new-name"
        class="wide"
        type="text"
        placeholder={t("document.untitled")}
        bind:value={name}
        autofocus
      />
      <label for="new-preset">{t("newDocument.preset")}</label>
      <select
        id="new-preset"
        class="wide"
        value={preset}
        onchange={(e) => choosePreset((e.currentTarget as HTMLSelectElement).value)}
      >
        <option value="custom">{t("newDocument.preset.custom")}</option>
        {#each PRESETS as p (p.id)}
          <option value={p.id}>{t(p.label)} ({p.width} × {p.height})</option>
        {/each}
      </select>
      <label for="new-width">{t("sizeDialog.width")}</label>
      <input id="new-width" type="number" min="1" max={MAX_SIDE} step="1" bind:value={width} />
      <span class="unit">{t("sizeDialog.unit.px")}</span>
      <label for="new-height">{t("sizeDialog.height")}</label>
      <input id="new-height" type="number" min="1" max={MAX_SIDE} step="1" bind:value={height} />
      <span class="unit">{t("sizeDialog.unit.px")}</span>
      <span class="label">{t("newDocument.orientation")}</span>
      <div class="orientation" role="radiogroup" aria-label={t("newDocument.orientation")}>
        <button
          type="button"
          role="radio"
          aria-checked={portrait}
          class:selected={portrait}
          title={t("newDocument.portrait")}
          aria-label={t("newDocument.portrait")}
          onclick={() => orient(true)}
        >
          <Icon name="portrait" size={16} />
        </button>
        <button
          type="button"
          role="radio"
          aria-checked={!portrait}
          class:selected={!portrait}
          title={t("newDocument.landscape")}
          aria-label={t("newDocument.landscape")}
          onclick={() => orient(false)}
        >
          <Icon name="landscape" size={16} />
        </button>
      </div>
      <label for="new-background">{t("newDocument.background")}</label>
      <select id="new-background" class="wide" bind:value={background}>
        {#each BACKGROUNDS as entry (entry.value)}
          <option value={entry.value}>{t(entry.label)}</option>
        {/each}
      </select>
    </div>
    <div class="buttons">
      <button type="submit" class="btn primary" disabled={!valid}>{t("sizeDialog.ok")}</button>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 480px;
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
    grid-template-columns: auto 1fr auto;
    align-items: center;
    align-self: start;
    gap: 8px;
  }

  .fields > label,
  .label {
    grid-column: 1;
    color: var(--text-muted);
  }

  .wide {
    grid-column: 2 / -1;
  }

  input,
  select {
    min-width: 0;
  }

  .unit {
    color: var(--text-muted);
  }

  .orientation {
    display: flex;
    gap: 4px;
  }

  .orientation button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
    background: var(--chrome);
    color: var(--text-muted);
  }

  .orientation button.selected {
    border-color: var(--accent);
    color: var(--text);
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
