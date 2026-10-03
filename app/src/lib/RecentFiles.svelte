<script lang="ts">
  // The welcome page's recent files (no tab open): a thumbnail of each, kept by the engine when
  // the file was opened or saved, and its name; a click opens it. Folders, archives and files
  // without a thumbnail show an icon.
  import { engine } from "./engine";
  import { t } from "./i18n/index.svelte";
  import Icon from "./Icon.svelte";
  import { recentLabels } from "./recent";

  let {
    paths,
    onopen,
  }: {
    /** Newest first. */
    paths: string[];
    onopen: (path: string) => void;
  } = $props();

  const labels = $derived(recentLabels(paths));

  /** Draws the thumbnail of `path` into the canvas; hides it when there is none. */
  function thumbnail(canvas: HTMLCanvasElement, path: string) {
    let current = path;
    const load = (path: string) => {
      canvas.hidden = true;
      engine.recentThumbnail(path).then(
        (image) => {
          if (current !== path) return;
          canvas.width = image.width;
          canvas.height = image.height;
          canvas.getContext("2d")?.putImageData(image, 0, 0);
          canvas.hidden = false;
        },
        () => {
          // No thumbnail: the icon behind shows.
        },
      );
    };
    load(path);
    return {
      update(next: string) {
        current = next;
        load(next);
      },
    };
  }
</script>

<section class="recent" aria-label={t("welcome.recent")}>
  <h2>{t("welcome.recent")}</h2>
  <ul>
    {#each paths as path, i (path)}
      <li>
        <button title={path} onclick={() => onopen(path)}>
          <span class="frame">
            <Icon name="folder" size={36} />
            <canvas use:thumbnail={path} hidden></canvas>
          </span>
          <span class="name">{labels[i]}</span>
        </button>
      </li>
    {/each}
  </ul>
</section>

<style>
  .recent {
    width: min(760px, 70vw);
    margin-top: 18px;
  }

  h2 {
    margin: 0 0 8px;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
  }

  ul {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(136px, 1fr));
    gap: 10px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  button {
    display: grid;
    gap: 6px;
    width: 100%;
    padding: 6px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: none;
    color: var(--text);
    font: inherit;
    text-align: center;
  }

  button:hover {
    border-color: var(--border-strong);
    background: var(--hover);
  }

  .frame {
    position: relative;
    display: grid;
    place-items: center;
    height: 112px;
    border-radius: 3px;
    background: var(--panel);
    color: var(--text-muted);
    overflow: hidden;
  }

  canvas {
    position: absolute;
    max-width: 100%;
    max-height: 100%;
    /* The thumbnail covers the icon once drawn. */
    background: var(--panel);
  }

  canvas[hidden] {
    display: none;
  }

  .name {
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
