<script lang="ts">
  // A filter's dialog (Filter > …, ADR 0034), laid out as Photoshop's: its settings on the left,
  // a number of pixels with a logarithmic slider below it; OK, Cancel and Preview on the right.
  // The app shows the settings live on the canvas; OK applies them (one undo entry), Cancel
  // leaves everything as it was. Enter applies, Esc cancels. The same dialog edits a filter
  // entry of a stack again.
  import { onMount, untrack } from "svelte";
  import type { FilterId } from "./engine";
  import { FILTERS, validValues } from "./filters";
  import { t } from "./i18n/index.svelte";

  let {
    filter,
    values,
    preview,
    onlive,
    onpreview,
    onok,
    oncancel,
  }: {
    filter: FilterId;
    /** The settings it opens with. */
    values: number[];
    /** The canvas shows the settings. */
    preview: boolean;
    /** Valid settings, each time they change, the first ones included. */
    onlive: (values: number[]) => void;
    onpreview: (preview: boolean) => void;
    onok: (values: number[]) => void;
    oncancel: () => void;
  } = $props();

  const params = $derived(FILTERS[filter].params);
  let current = $state<number[]>(untrack(() => [...values]));
  const valid = $derived(validValues(filter, current));

  $effect(() => {
    const shown = [...current];
    if (untrack(() => validValues(filter, shown))) untrack(() => onlive(shown));
  });

  let dialog: HTMLDialogElement;

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onok([...current]);
  }

  /** A slider position (0–1000) for value `v` between `min` and `max`, logarithmically. */
  const position = (v: number, min: number, max: number) =>
    (Math.log(Math.max(v, min) / min) / Math.log(max / min)) * 1000;
  const fromPosition = (p: number, min: number, max: number, decimals: number) =>
    Number((min * Math.pow(max / min, p / 1000)).toFixed(decimals));
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="filter-title"
  oncancel={(e) => {
    e.preventDefault();
    oncancel();
  }}
>
  <header id="filter-title">{t(`filter.${filter}`)}</header>
  <form onsubmit={submit}>
    <div class="settings">
      {#each params as param, i (param.key)}
        <div class="row">
          <label for="filter-{param.key}">{t(`filter.${filter}.${param.key}`)}</label>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            id="filter-{param.key}"
            type="number"
            min={param.min}
            max={param.max}
            step="any"
            autofocus={i === 0}
            bind:value={current[i]}
          />
          <span>{t("filter.pixels")}</span>
        </div>
        <input
          class="slider"
          type="range"
          min="0"
          max="1000"
          step="1"
          aria-label={t(`filter.${filter}.${param.key}`)}
          value={position(current[i], param.min, param.max)}
          oninput={(e) =>
            (current[i] = fromPosition(
              e.currentTarget.valueAsNumber,
              param.min,
              param.max,
              param.decimals,
            ))}
        />
      {/each}
    </div>
    <div class="buttons">
      <button type="submit" class="btn primary" disabled={!valid}>{t("sizeDialog.ok")}</button>
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
    width: 340px;
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
    padding: 10px;
  }

  .settings {
    display: grid;
    align-content: start;
    gap: 6px;
    min-width: 0;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .row input {
    width: 70px;
  }

  .slider {
    width: 100%;
  }

  .buttons {
    display: grid;
    align-content: start;
    gap: 6px;
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
