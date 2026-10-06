// Version-free copies of a release's installers, so that the README's download links
// (`releases/latest/download/<file>`) always give the latest version. release.yml runs it once
// the installers are attached to the draft. Running it again replaces the copies.
//
//   GH_TOKEN=… node scripts/stable-downloads.mjs <owner/repo> <release id> <tag>

import { fileURLToPath } from "node:url";

/** The installers the README links to: [name in the release (`{v}`: version), copy's name]. */
export const INSTALLERS = [
  ["SlopShop_{v}_x64-setup.exe", "SlopShop_x64-setup.exe"],
  ["SlopShop_{v}_x64_en-US.msi", "SlopShop_x64_en-US.msi"],
  ["SlopShop_{v}_aarch64.dmg", "SlopShop_aarch64.dmg"],
  ["SlopShop_{v}_x64.dmg", "SlopShop_x64.dmg"],
  ["SlopShop_{v}_amd64.deb", "SlopShop_amd64.deb"],
  ["SlopShop-{v}-1.x86_64.rpm", "SlopShop.x86_64.rpm"],
  ["SlopShop_{v}_amd64.AppImage", "SlopShop_amd64.AppImage"],
];

/**
 * The copies to make in a release of `version` whose files are `names`: [file, copy] pairs.
 * Throws when an installer is missing.
 * @param {string[]} names
 * @param {string} version
 * @returns {[string, string][]}
 */
export function stableCopies(names, version) {
  const present = new Set(names);
  return INSTALLERS.map(([pattern, stable]) => {
    const file = pattern.replace("{v}", version);
    if (!present.has(file)) throw new Error(`${file} is not attached to the release`);
    return [file, stable];
  });
}

/**
 * Copy the installers of release `id` (tagged `tag`) of `repo` under their version-free names,
 * replacing earlier copies.
 * @param {string} repo
 * @param {string} id
 * @param {string} tag
 * @param {string} token
 */
async function copyInstallers(repo, id, tag, token) {
  const authorization = `Bearer ${token}`;
  /** @param {string} url @param {RequestInit} [init] */
  const request = async (url, init = {}) => {
    const response = await fetch(url, {
      ...init,
      headers: { authorization, accept: "application/vnd.github+json", ...init.headers },
    });
    if (!response.ok) {
      throw new Error(
        `${init.method ?? "GET"} ${url}: ${response.status} ${await response.text()}`,
      );
    }
    return response;
  };
  const api = `https://api.github.com/repos/${repo}/releases`;
  /** @type {{ id: number, name: string }[]} */
  const assets = await (await request(`${api}/${id}/assets?per_page=100`)).json();
  const byName = new Map(assets.map((asset) => [asset.name, asset.id]));
  for (const [file, stable] of stableCopies([...byName.keys()], tag.replace(/^v/, ""))) {
    const earlier = byName.get(stable);
    if (earlier !== undefined) await request(`${api}/assets/${earlier}`, { method: "DELETE" });
    // The download is redirected to storage, which fetch reaches without the token.
    const download = await request(`${api}/assets/${byName.get(file)}`, {
      headers: { accept: "application/octet-stream" },
    });
    const bytes = await download.arrayBuffer();
    const upload = `https://uploads.github.com/repos/${repo}/releases/${id}/assets`;
    await request(`${upload}?name=${encodeURIComponent(stable)}`, {
      method: "POST",
      headers: { "content-type": "application/octet-stream" },
      body: bytes,
    });
    console.log(`${file} -> ${stable}`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [repo, id, tag] = process.argv.slice(2);
  const token = process.env.GH_TOKEN;
  if (!repo || !id || !tag || !token) {
    throw new Error("usage: GH_TOKEN=… stable-downloads.mjs <owner/repo> <release id> <tag>");
  }
  await copyInstallers(repo, id, tag, token);
}
