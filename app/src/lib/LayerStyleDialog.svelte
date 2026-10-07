<script lang="ts">
  // Layer > Layer Style (ADR 0032), laid out as Photoshop's Layer Style dialog: Blending
  // Options and the effects on the left (a checkbox turns an effect on or off, a click shows
  // its settings), the selected one's settings in the middle, OK and Cancel on the right. The
  // canvas shows every change at once (the app sends each as a live edit); OK keeps them as one
  // undo entry, Cancel takes them back. Enter applies, Esc cancels.
  import { onMount, untrack } from "svelte";
  import { BLEND_MODE_GROUPS, type LayerStyle } from "./engine";
  import { srgbToHex } from "./color";
  import { EFFECTS, PLAIN, withEffect, type EffectColor, type EffectId } from "./layerStyle";
  import SliderField from "./SliderField.svelte";
  import StyleEffectFields from "./StyleEffectFields.svelte";
  import GradientEditor from "./GradientEditor.svelte";
  import SourceThumbnail from "./SourceThumbnail.svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  /** What the left list selects: Blending Options or an effect. */
  export type StylePage = "blending" | EffectId;

  let {
    documentId,
    style,
    page,
    onchange,
    onpage,
    onpickcolor,
    onpickpattern,
    onok,
    oncancel,
  }: {
    /** The document, whose sources the Pattern Overlay's thumbnail shows. */
    documentId: number;
    /** The style as the dialog opens (or opens again after the color picker). */
    style: LayerStyle | null;
    page: StylePage;
    /** A setting changed: the style it makes (the canvas follows it). */
    onchange: (style: LayerStyle) => void;
    /** Another page shown. */
    onpage: (page: StylePage) => void;
    /** An effect's color swatch clicked: the app shows the picker, then this dialog again. */
    onpickcolor: (effect: EffectId, color?: EffectColor) => void;
    /** A Pattern Overlay's pattern to choose (ADR 0042): the app shows the picker, then this
     * dialog again with it. */
    onpickpattern: () => void;
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
    // A Pattern Overlay starts with its pattern chosen.
    if (id === "patternOverlay" && enabled && !draft.patternOverlay) {
      onpickpattern();
      return;
    }
    const next = withEffect(draft, id, enabled);
    draft[id] = next[id] as never;
    if (enabled) onpage(id);
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    onok();
  }

  const POSITIONS = ["outside", "inside", "center"] as const;
  const BEVEL_STYLES = ["innerBevel", "outerBevel", "emboss", "pillowEmboss"] as const;
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

{#snippet swatch(color: number[], effect: EffectId, which: EffectColor = "color")}
  <button
    type="button"
    class="swatch"
    style:background={srgbToHex(color)}
    title={t(which === "color" ? "style.color" : `style.${which}`)}
    aria-label={t(which === "color" ? "style.color" : `style.${which}`)}
    onclick={() => onpickcolor(effect, which)}
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
  <header id="style-title" {@attach movable("style")}>{t("style.title")}</header>
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
      {:else if page === "bevel"}
        <h3>{t("style.bevel")}</h3>
        {#if draft.bevel}
          <label class="field">
            <span>{t("style.bevelStyle")}</span>
            <select bind:value={draft.bevel.style}>
              {#each BEVEL_STYLES as id (id)}
                <option value={id}>{t(`style.bevelStyle.${id}`)}</option>
              {/each}
            </select>
          </label>
          <SliderField
            label={t("style.depth")}
            bind:value={draft.bevel.depth}
            min={1}
            max={1000}
            unit="%"
          />
          <label class="field">
            <span>{t("style.direction")}</span>
            <select
              value={draft.bevel.up ? "up" : "down"}
              onchange={(e) => (draft.bevel!.up = e.currentTarget.value === "up")}
            >
              <option value="up">{t("style.direction.up")}</option>
              <option value="down">{t("style.direction.down")}</option>
            </select>
          </label>
          <SliderField
            label={t("style.size")}
            bind:value={draft.bevel.size}
            min={0}
            max={250}
            unit="px"
          />
          <SliderField
            label={t("style.soften")}
            bind:value={draft.bevel.soften}
            min={0}
            max={16}
            unit="px"
          />
          <SliderField
            label={t("style.angle")}
            bind:value={draft.bevel.angle}
            min={-180}
            max={180}
            unit="°"
          />
          <SliderField
            label={t("style.altitude")}
            bind:value={draft.bevel.altitude}
            min={0}
            max={90}
            unit="°"
          />
          {@render modes(
            draft.bevel.highlightMode,
            (m) => (draft.bevel!.highlightMode = m as never),
            t("style.highlightMode"),
          )}
          <div class="field">
            <span>{t("style.highlightColor")}</span>
            {@render swatch(draft.bevel.highlightColor, "bevel", "highlightColor")}
          </div>
          <SliderField
            label={t("style.opacity")}
            bind:value={draft.bevel.highlightOpacity}
            min={0}
            max={100}
            unit="%"
            factor={100}
          />
          {@render modes(
            draft.bevel.shadowMode,
            (m) => (draft.bevel!.shadowMode = m as never),
            t("style.shadowMode"),
          )}
          <div class="field">
            <span>{t("style.shadowColor")}</span>
            {@render swatch(draft.bevel.shadowColor, "bevel", "shadowColor")}
          </div>
          <SliderField
            label={t("style.opacity")}
            bind:value={draft.bevel.shadowOpacity}
            min={0}
            max={100}
            unit="%"
            factor={100}
          />
        {:else}
          <p class="empty">{t("style.off")}</p>
        {/if}
      {:else if page === "satin"}
        <h3>{t("style.satin")}</h3>
        {#if draft.satin}
          {@render modes(
            draft.satin.mode,
            (m) => (draft.satin!.mode = m as never),
            t("style.mode"),
          )}
          <div class="field">
            <span>{t("style.color")}</span>
            {@render swatch(draft.satin.color, "satin")}
          </div>
          <SliderField
            label={t("style.opacity")}
            bind:value={draft.satin.opacity}
            min={0}
            max={100}
            unit="%"
            factor={100}
          />
          <SliderField
            label={t("style.angle")}
            bind:value={draft.satin.angle}
            min={-180}
            max={180}
            unit="°"
          />
          <SliderField
            label={t("style.distance")}
            bind:value={draft.satin.distance}
            min={0}
            max={250}
            unit="px"
          />
          <SliderField
            label={t("style.size")}
            bind:value={draft.satin.size}
            min={0}
            max={250}
            unit="px"
          />
          <label class="check">
            <input type="checkbox" bind:checked={draft.satin.invert} />
            {t("style.invert")}
          </label>
        {:else}
          <p class="empty">{t("style.off")}</p>
        {/if}
      {:else if page === "gradientOverlay"}
        <h3>{t("style.gradientOverlay")}</h3>
        {#if draft.gradientOverlay}
          {@render modes(
            draft.gradientOverlay.mode,
            (m) => (draft.gradientOverlay!.mode = m as never),
            t("style.mode"),
          )}
          <SliderField
            label={t("style.opacity")}
            bind:value={draft.gradientOverlay.opacity}
            min={0}
            max={100}
            unit="%"
            factor={100}
          />
          <div class="field gradient">
            <span>{t("style.gradient")}</span>
            <GradientEditor
              stops={draft.gradientOverlay.stops}
              onlive={(stops) => (draft.gradientOverlay!.stops = stops)}
              onend={() => {}}
              onapply={(stops) => (draft.gradientOverlay!.stops = stops)}
            />
          </div>
          <label class="check">
            <input type="checkbox" bind:checked={draft.gradientOverlay.reverse} />
            {t("style.reverse")}
          </label>
          <label class="field">
            <span>{t("style.gradientShape")}</span>
            <select bind:value={draft.gradientOverlay.shape}>
              <option value="linear">{t("options.gradient.linear")}</option>
              <option value="radial">{t("options.gradient.radial")}</option>
            </select>
          </label>
          <label class="check">
            <input type="checkbox" bind:checked={draft.gradientOverlay.align} />
            {t("style.alignWithLayer")}
          </label>
          <SliderField
            label={t("style.angle")}
            bind:value={draft.gradientOverlay.angle}
            min={-180}
            max={180}
            unit="°"
          />
          <SliderField
            label={t("style.scale")}
            bind:value={draft.gradientOverlay.scale}
            min={10}
            max={150}
            unit="%"
          />
        {:else}
          <p class="empty">{t("style.off")}</p>
        {/if}
      {:else if page === "patternOverlay"}
        <h3>{t("style.patternOverlay")}</h3>
        {#if draft.patternOverlay}
          {@render modes(
            draft.patternOverlay.mode,
            (m) => (draft.patternOverlay!.mode = m as never),
            t("style.mode"),
          )}
          <SliderField
            label={t("style.opacity")}
            bind:value={draft.patternOverlay.opacity}
            min={0}
            max={100}
            unit="%"
            factor={100}
          />
          <div class="field">
            <span>{t("style.pattern")}</span>
            <button
              type="button"
              class="pattern"
              title={t("patterns.replace")}
              aria-label={t("patterns.replace")}
              onclick={onpickpattern}
            >
              <SourceThumbnail {documentId} source={draft.patternOverlay.source} size={40} />
            </button>
          </div>
          <SliderField
            label={t("style.scale")}
            bind:value={draft.patternOverlay.scale}
            min={1}
            max={1000}
            unit="%"
            factor={100}
          />
          <SliderField
            label={t("style.angle")}
            bind:value={draft.patternOverlay.angle}
            min={-180}
            max={180}
            unit="°"
          />
          <label class="check">
            <input type="checkbox" bind:checked={draft.patternOverlay.link} />
            {t("style.linkWithLayer")}
          </label>
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

  .pattern {
    display: inline-flex;
    padding: 0;
    border: 1px solid var(--border-strong);
    background: none;
    cursor: pointer;
  }
</style>
