// The update manifest installed copies are offered when a release is published (ADR 0039,
// .github/workflows/updates.yml). tauri-action writes latest.json while the release is still a
// draft, so its packages are named by GitHub API addresses (`…/releases/assets/<id>`) and it has
// no notes. Once the release is published, each package gets its download address
// (`…/releases/download/<tag>/<file>`), and the notes are the release's text above its first
// `## ` heading (what changed; the rest is the same in every release).
//
//   node scripts/update-manifest.mjs <latest.json> <release.json> <tag>   (writes to stdout)
//
// `release.json` is the published release, as `gh api repos/<repo>/releases/tags/<tag>` gives.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/**
 * @typedef {{ url: string, signature: string }} Platform
 * @typedef {{ version: string, notes?: string, pub_date?: string,
 *   platforms: Record<string, Platform> }} Manifest
 * @typedef {{ body?: string | null,
 *   assets: { id: number, browser_download_url: string }[] }} Release
 */

/**
 * The part of a release's text installed copies show: above its first `## ` heading.
 * @param {string | null | undefined} body
 */
export function releaseNotes(body) {
  const text = (body ?? "").replace(/\r\n/g, "\n");
  const heading = text.search(/^## /m);
  return (heading < 0 ? text : text.slice(0, heading)).trim();
}

/**
 * `manifest` with download addresses of `release` (tagged `tag`) and its notes. Throws when a
 * package is not one of the release's, or the versions differ.
 * @param {Manifest} manifest
 * @param {Release} release
 * @param {string} tag
 * @returns {Manifest}
 */
export function offerManifest(manifest, release, tag) {
  if (`v${manifest.version}` !== tag) {
    throw new Error(`latest.json is for version ${manifest.version}, not ${tag}`);
  }
  const addresses = new Map(release.assets.map((a) => [String(a.id), a.browser_download_url]));
  const prefix = `/releases/download/${tag}/`;
  /** @type {Record<string, Platform>} */
  const platforms = {};
  for (const [name, platform] of Object.entries(manifest.platforms)) {
    const id = /\/releases\/assets\/(\d+)$/.exec(platform.url)?.[1];
    const url = id === undefined ? platform.url : addresses.get(id);
    if (!url?.includes(prefix)) {
      throw new Error(`${name}: ${platform.url} is not a package of ${tag}`);
    }
    platforms[name] = { ...platform, url };
  }
  return { ...manifest, notes: releaseNotes(release.body), platforms };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [manifestPath, releasePath, tag] = process.argv.slice(2);
  if (!manifestPath || !releasePath || !tag) {
    throw new Error("usage: update-manifest.mjs <latest.json> <release.json> <tag>");
  }
  const read = (/** @type {string} */ path) => JSON.parse(readFileSync(path, "utf8"));
  const offered = offerManifest(read(manifestPath), read(releasePath), tag);
  process.stdout.write(`${JSON.stringify(offered, null, 2)}\n`);
}
