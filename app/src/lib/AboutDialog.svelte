<script lang="ts">
  // Help > About SlopShop: the name, the version as built, the license and the project's page,
  // all from the engine (the workspace's manifest).
  import { onMount } from "svelte";
  import { engine, type AppInfo } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let { info, onclose }: { info: AppInfo; onclose: () => void } = $props();

  let dialog: HTMLDialogElement;

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="about-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      onclose();
    }}
  >
    <header id="about-title" {@attach movable("about")}>{t("about.title")}</header>
    <div class="body">
      <h1>SlopShop</h1>
      <p>{t("about.version", { version: info.version })}</p>
      <p class="muted">{t("about.tagline")}</p>
      <dl>
        <dt>{t("about.license")}</dt>
        <dd>{info.license}</dd>
        <dt>{t("about.project")}</dt>
        <dd>
          <button type="button" class="link" onclick={() => void engine.openProjectPage("home")}>
            {info.repository.replace(/^https:\/\//, "")}
          </button>
        </dd>
      </dl>
    </div>
    <footer>
      <!-- svelte-ignore a11y_autofocus -->
      <button type="submit" class="btn primary" autofocus>{t("sizeDialog.ok")}</button>
    </footer>
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

  .body {
    display: grid;
    gap: 4px;
    padding: 14px 12px;
  }

  h1 {
    margin: 0 0 2px;
    font-size: 1.6em;
    font-weight: 600;
  }

  p {
    margin: 0;
  }

  .muted {
    color: var(--text-muted);
  }

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 14px;
    margin: 10px 0 0;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    min-width: 0;
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    text-align: left;
    text-decoration: underline;
    overflow-wrap: anywhere;
    cursor: pointer;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
