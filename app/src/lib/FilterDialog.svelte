<script lang="ts" module>
  // Show More Options as it was left last, for the session (Photoshop keeps it).
  let moreOptions = false;
</script>

<script lang="ts">
  // A filter's dialog (Filter > …, ADR 0034), laid out as Photoshop's: its settings on the left,
  // each a number with its unit and a slider below it; OK, Cancel and Preview on the right.
  // The app shows the settings live on the canvas; OK applies them (one undo entry), Cancel
  // leaves everything as it was. Enter applies, Esc cancels. The same dialog edits a filter
  // entry of a stack again.
  import { onMount, untrack } from "svelte";
  import { movable } from "./dialogDrag";
  import type { FilterId } from "./engine";
  import {
    FILTERS,
    KERNEL_SIDE,
    MAX_WEIGHT,
    centerAt,
    newSeed,
    nudgedCenter,
    shownParams,
    sliderPosition,
    sliderValue,
    validValues,
  } from "./filters";
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
  // Show More Options (Shadows/Highlights): the settings marked `more` shown too.
  let more = $state(moreOptions);
  const shown = $derived(new Set(shownParams(filter, more)));
  // A heading above the first setting shown of each group.
  const headed = $derived(
    new Set(
      [...shown].filter((i, n, all) => {
        const group = (p: (typeof params)[number] | undefined) =>
          p?.kind === "number" ? p.group : undefined;
        return group(params[i]) !== undefined && group(params[i]) !== group(params[all[n - 1]]);
      }),
    ),
  );
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
    // Stopped before it reaches the dialog's own handlers: a center's square takes its arrows
    // here.
    const isolate = (e: KeyboardEvent) => {
      e.stopPropagation();
      const square = e.target instanceof HTMLElement ? e.target.closest("[data-center]") : null;
      if (square instanceof HTMLElement) nudgeCenter(e, Number(square.dataset.center));
    };
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });

  // A center picked in its square (Radial Blur's Blur Center): values `i` and `i + 1`.
  function pickCenter(e: PointerEvent, i: number) {
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    [current[i], current[i + 1]] = centerAt(box, e.clientX, e.clientY);
  }

  function nudgeCenter(e: KeyboardEvent, i: number) {
    const moved = nudgedCenter([current[i], current[i + 1]], e.key, e.shiftKey);
    if (!moved) return;
    e.preventDefault();
    [current[i], current[i + 1]] = moved;
  }

  // The square's lines: circles around the center for a spin, rays from it for a zoom (as
  // Photoshop draws them), by the filter's "method" setting.
  const zooms = $derived(current[params.findIndex((p) => p.key === "method")] === 1);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onok([...current]);
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="filter-title"
  oncancel={(e) => {
    e.preventDefault();
    oncancel();
  }}
>
  <header id="filter-title" {@attach movable("filter")}>{t(`filter.${filter}`)}</header>
  <form onsubmit={submit}>
    <div class="settings">
      {#each params as param, i (param.key)}
        {#if !shown.has(i)}
          <!-- Hidden without Show More Options. -->
        {:else if param.kind === "choice"}
          <fieldset>
            <legend>{t(param.label)}</legend>
            {#each param.options as option, n (option)}
              <label class="check">
                <input
                  type="radio"
                  name="filter-{param.key}"
                  checked={current[i] === n}
                  onchange={() => (current[i] = n)}
                />
                {t(option)}
              </label>
            {/each}
          </fieldset>
        {:else if param.kind === "check"}
          <label class="check">
            <input
              type="checkbox"
              checked={current[i] === 1}
              onchange={(e) => (current[i] = e.currentTarget.checked ? 1 : 0)}
            />
            {t(param.label)}
          </label>
        {:else if param.kind === "seed"}
          {#if param.randomize}
            <!-- Another seed: other waves, shown live. -->
            <button type="button" class="btn" onclick={() => (current[i] = newSeed())}>
              {t(param.randomize)}
            </button>
          {/if}
        {:else if param.kind === "center"}
          {#if param.axis === "x"}
            <div class="center-setting">
              <span id="filter-{param.key}">{t(param.label)}</span>
              <div
                class="center"
                role="slider"
                tabindex="0"
                aria-labelledby="filter-{param.key}"
                aria-valuetext="{current[i]}, {current[i + 1]}"
                aria-valuenow={current[i]}
                aria-valuemin="0"
                aria-valuemax="100"
                onpointerdown={(e) => {
                  (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
                  pickCenter(e, i);
                }}
                onpointermove={(e) => {
                  if (e.buttons & 1) pickCenter(e, i);
                }}
                data-center={i}
              >
                <svg viewBox="0 0 100 100" aria-hidden="true">
                  {#if zooms}
                    {#each Array.from({ length: 16 }, (_, n) => (n * Math.PI) / 8) as a (a)}
                      <line
                        x1={current[i]}
                        y1={current[i + 1]}
                        x2={current[i] + Math.cos(a) * 150}
                        y2={current[i + 1] + Math.sin(a) * 150}
                      />
                    {/each}
                  {:else}
                    {#each [12, 24, 36, 48, 60, 72, 84, 96, 108, 120, 132] as r (r)}
                      <circle cx={current[i]} cy={current[i + 1]} {r} />
                    {/each}
                  {/if}
                </svg>
              </div>
            </div>
          {/if}
        {:else if param.kind === "weight"}
          {#if param.cell === 0}
            <!-- Custom's weights: the grid of the pixels around, row by row. -->
            <fieldset>
              <legend>{t("filter.custom.weights")}</legend>
              <div class="kernel">
                {#each Array.from({ length: KERNEL_SIDE * KERNEL_SIDE }, (_, n) => n) as n (n)}
                  <!-- svelte-ignore a11y_autofocus -->
                  <input
                    type="number"
                    min={-MAX_WEIGHT}
                    max={MAX_WEIGHT}
                    step="1"
                    aria-label={t("filter.custom.weight", {
                      row: Math.floor(n / KERNEL_SIDE) + 1,
                      column: (n % KERNEL_SIDE) + 1,
                    })}
                    autofocus={n === (KERNEL_SIDE * KERNEL_SIDE - 1) / 2}
                    bind:value={current[i + n]}
                  />
                {/each}
              </div>
            </fieldset>
          {/if}
        {:else if param.kind === "number"}
          {#if param.group && headed.has(i)}
            <div class="group">{t(param.group)}</div>
          {/if}
          <div class="row">
            <label for="filter-{param.key}">{t(param.label)}</label>
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
            {#if param.unit}<span>{t(param.unit)}</span>{/if}
          </div>
          <input
            class="slider"
            type="range"
            min="0"
            max="1000"
            step="1"
            aria-label={t(param.label)}
            value={sliderPosition(param, current[i])}
            oninput={(e) => (current[i] = sliderValue(param, e.currentTarget.valueAsNumber))}
          />
        {/if}
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
      {#if FILTERS[filter].more}
        <label class="check">
          <input
            type="checkbox"
            checked={more}
            onchange={(e) => {
              more = (e.currentTarget as HTMLInputElement).checked;
              moreOptions = more;
            }}
          />
          {t("filter.moreOptions")}
        </label>
      {/if}
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

  /* A group's heading (Shadows/Highlights' Shadows, Highlights, Adjustments). */
  .group {
    margin-top: 4px;
    font-weight: 600;
  }

  fieldset {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 4px 8px 6px;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
  }

  /* Custom's 5 × 5 weights. */
  .kernel {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 3px;
  }

  .kernel input {
    width: 100%;
    min-width: 0;
    text-align: center;
  }

  .buttons {
    display: grid;
    align-content: start;
    gap: 6px;
  }

  .buttons .btn {
    min-width: 80px;
  }

  .center-setting {
    display: grid;
    gap: 4px;
    justify-items: start;
  }

  /* Photoshop's Blur Center: a square of the frame, the center picked in it. */
  .center {
    width: 120px;
    height: 120px;
    border: 1px solid var(--border-dark);
    background: var(--field);
    cursor: crosshair;
    touch-action: none;
  }

  .center svg {
    display: block;
    width: 100%;
    height: 100%;
    overflow: hidden;
    fill: none;
    stroke: var(--text);
    stroke-opacity: 0.6;
    stroke-width: 0.8;
    pointer-events: none;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }
</style>
