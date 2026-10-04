<script lang="ts">
  // Layer > Layer Style (ADR 0032), laid out as Photoshop's Layer Style dialog: Blending
  // Options and the effects on the left (a checkbox turns an effect on or off, a click shows
  // its settings), the selected one's settings in the middle, OK and Cancel on the right. The
  // canvas shows every change at once (the app sends each as a live edit); OK keeps them as one
  // undo entry, Cancel takes them back. Enter applies, Esc cancels.
  import { onMount, untrack } from "svelte";
  import { BLEND_MODE_GROUPS, type LayerStyle } from "./engine";
  import { srgbToHex } from "./color";
  import { EFFECTS, PLAIN, withEffect, type EffectId } from "./layerStyle";
  import SliderField from "./SliderField.svelte";
  import StyleEffectFields from "./StyleEffectFields.svelte";
  import { t } from "./i18n/index.svelte";

  /** What the left list selects: Blending Options or an effect. */
  export type StylePage = "blending" | EffectId;

  let {
    style,
    page,
    onchange,
    onpage,
    onpickcolor,
    onok,
    oncancel,
  }: {
    /** The style as the dialog opens (or opens again after the color picker). */
    style: LayerStyle | null;
    page: StylePage;
    /** A setting changed: the style it makes (the canvas follows it). */
    onchange: (style: LayerStyle) => void;
    /** Another page shown. */
    onpage: (page: StylePage) => void;
    /** An effect's color swatch clicked: the app shows the picker, then this dialog again. */
    onpickcolor: (effect: EffectId) => void;
    onok: () => void;
    oncancel: () => void;
  } = $props();

  /** The style being edited (the settings bind to it). */
  let draft = $state<LayerStyle>(untrack(() => structuredClone($state.snapshot(style) ?? PLAIN)));
  let dialog: HTMLDialogElement;
  let form: HTMLFormElement;

  // Every change of the settings is sent, not the first state.
  let first = true;
  $effect(() => {
    const snapshot = $state.snapshot(draft);
    if (first) {
      first = false;
      return;
    }
    untrack(() => onchange(snapshot));
  });

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

  /** An effect's checkbox: on (added at its defaults if it was not there) or off. */
  function toggle(id: EffectId, enabled: boolean) {
    const next = withEffect(draft, id, enabled);
    draft[id] = next[id] as never;
    if (enabled) onpage(id);
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    onok();
  }

  const POSITIONS = ["outside", "inside", "center"] as const;
</script>

{#snippet modes(value: string, set: (mode: string) => void, label: string)}
  <label class="field">
    <span>{label}</span>
    <select {value} onchange={(e) => set((e.currentTarget as HTMLSelectElement).value)}>
      {#each BLEND_MODE_GROUPS as group, i (i)}
        {#if i > 0}<hr />{/if}
        {#each group as mode (mode)}
          <option value={mode}>{t(`blendMode.${mode}`)}</option>
        {/each}
      {/each}
    </select>
  </label>
{/snippet}

{#snippet swatch(color: number[], effect: EffectId)}
  <button
    type="button"
    class="swatch"
    style:background={srgbToHex(color)}
    title={t("style.color")}
    aria-label={t("style.color")}
    onclick={() => onpickcolor(effect)}
  ></button>
{/snippet}

<dialog
  bind:this={dialog}
  aria-labelledby="style-title"
  oncancel={(e) => {
    e.preventDefault();
    oncancel();
  }}
>
  <header id="style-title">{t("style.title")}</header>
  <form bind:this={form} onsubmit={submit}>
    <ul class="pages" role="listbox" aria-label={t("style.title")}>
      <li
        role="option"
        aria-selected={page === "blending"}
        class:current={page === "blending"}
        onclick={() => onpage("blending")}
        onkeydown={() => {}}
      >
        <span class="label">{t("style.blending")}</span>
      </li>
      {#each EFFECTS as effect (effect.id)}
        <li
          role="option"
          aria-selected={page === effect.id}
          class:current={page === effect.id}
          onclick={() => onpage(effect.id)}
          onkeydown={() => {}}
        >
          <input
            type="checkbox"
            aria-label={t(effect.label)}
            checked={draft[effect.id]?.enabled ?? false}
            onclick={(e) => e.stopPropagation()}
            onchange={(e) => toggle(effect.id, (e.currentTarget as HTMLInputElement).checked)}
          />
          <span class="label">{t(effect.label)}</span>
        </li>
      {/each}
    </ul>

    <section
      class="settings"
      aria-label={t(page === "blending" ? "style.blending" : `style.${page}`)}
    >
      {#if page === "blending"}
        <h3>{t("style.blending")}</h3>
        <SliderField
          label={t("style.fillOpacity")}
          bind:value={draft.fillOpacity}
          min={0}
          max={100}
          unit="%"
          factor={100}
        />
      {:else if page === "stroke"}
        <h3>{t("style.stroke")}</h3>
        {#if draft.stroke}
          <SliderField
            label={t("style.size")}
            bind:value={draft.stroke.size}
            min={1}
            max={250}
            unit="px"
          />
          <label class="field">
            <span>{t("style.position")}</span>
            <select bind:value={draft.stroke.position}>
              {#each POSITIONS as position (position)}
                <option value={position}>{t(`style.position.${position}`)}</option>
              {/each}
            </select>
          </label>
          {@render modes(
            draft.stroke.mode,
            (m) => (draft.stroke!.mode = m as never),
            t("style.mode"),
          )}
          <SliderField
            label={t("style.opacity")}
            bind:value={draft.stroke.opacity}
            min={0}
            max={100}
            unit="%"
            factor={100}
          />
          <div class="field">
            <span>{t("style.color")}</span>
            {@render swatch(draft.stroke.color, "stroke")}
          </div>
        {:else}
          <p class="empty">{t("style.off")}</p>
        {/if}
      {:else if page === "colorOverlay"}
        <h3>{t("style.colorOverlay")}</h3>
        {#if draft.colorOverlay}
          {@render modes(
            draft.colorOverlay.mode,
            (m) => (draft.colorOverlay!.mode = m as never),
            t("style.mode"),
          )}
          <div class="field">
            <span>{t("style.color")}</span>
            {@render swatch(draft.colorOverlay.color, "colorOverlay")}
          </div>
          <SliderField
            label={t("style.opacity")}
            bind:value={draft.colorOverlay.opacity}
            min={0}
            max={100}
            unit="%"
            factor={100}
          />
        {:else}
          <p class="empty">{t("style.off")}</p>
        {/if}
      {:else}
        <!-- A shadow or a glow, outside or inside the shape. -->
        <h3>{t(`style.${page}`)}</h3>
        {#if page === "dropShadow" && draft.dropShadow}
          <StyleEffectFields
            bind:effect={draft.dropShadow}
            inside={false}
            onpickcolor={() => onpickcolor("dropShadow")}
          />
        {:else if page === "innerShadow" && draft.innerShadow}
          <StyleEffectFields
            bind:effect={draft.innerShadow}
            inside
            onpickcolor={() => onpickcolor("innerShadow")}
          />
        {:else if page === "outerGlow" && draft.outerGlow}
          <StyleEffectFields
            bind:effect={draft.outerGlow}
            inside={false}
            onpickcolor={() => onpickcolor("outerGlow")}
          />
        {:else if page === "innerGlow" && draft.innerGlow}
          <StyleEffectFields
            bind:effect={draft.innerGlow}
            inside
            onpickcolor={() => onpickcolor("innerGlow")}
          />
        {:else}
          <p class="empty">{t("style.off")}</p>
        {/if}
      {/if}
    </section>

    <div class="buttons">
      <!-- svelte-ignore a11y_autofocus -->
      <button type="submit" class="btn primary" autofocus>{t("sizeDialog.ok")}</button>
      <button type="button" class="btn" onclick={oncancel}>{t("sizeDialog.cancel")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 560px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  /* The canvas behind shows the effects: no darkening. */
  dialog::backdrop {
    background: transparent;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  /* Photoshop's layout: the list, the settings, OK and Cancel stacked on the right. */
  form {
    display: grid;
    grid-template-columns: 150px 1fr auto;
    gap: 10px;
    padding: 8px 10px 10px;
  }

  .pages {
    margin: 0;
    padding: 2px 0;
    list-style: none;
    border: 1px solid var(--border-dark);
    background: var(--panel-header);
  }

  .pages li {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 6px;
    cursor: default;
  }

  .pages li.current {
    background: var(--accent);
    color: var(--accent-text, #fff);
  }

  .settings {
    display: grid;
    align-content: start;
    gap: 6px;
    min-width: 0;
  }

  h3 {
    margin: 0 0 4px;
    font-size: 1em;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .field > span {
    min-width: 90px;
  }

  .swatch {
    width: 40px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 2px;
    cursor: pointer;
  }

  .empty {
    color: var(--text-muted);
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
