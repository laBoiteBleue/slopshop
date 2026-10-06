import { describe, expect, test } from "vitest";
import { offerManifest, releaseNotes } from "../scripts/update-manifest.mjs";

const API = "https://api.github.com/repos/laBoiteBleue/slopshop/releases/assets";
const DOWNLOAD = "https://github.com/laBoiteBleue/slopshop/releases/download/v0.1.2";

/** latest.json as tauri-action writes it into a draft release. */
const draft = {
  version: "0.1.2",
  notes: "",
  pub_date: "2026-10-07T10:00:00.000Z",
  platforms: {
    "windows-x86_64": { url: `${API}/11`, signature: "sig-nsis" },
    "windows-x86_64-msi": { url: `${API}/12`, signature: "sig-msi" },
    "linux-x86_64": { url: `${API}/13`, signature: "sig-appimage" },
  },
};

/** The release once published. */
const published = {
  body: "An early build.\r\n\r\n- Faster brushes.\r\n\r\n## Which file to download\r\n\r\nA table.",
  assets: [
    { id: 11, browser_download_url: `${DOWNLOAD}/SlopShop_0.1.2_x64-setup.exe` },
    { id: 12, browser_download_url: `${DOWNLOAD}/SlopShop_0.1.2_x64_en-US.msi` },
    { id: 13, browser_download_url: `${DOWNLOAD}/SlopShop_0.1.2_amd64.AppImage` },
    { id: 14, browser_download_url: `${DOWNLOAD}/latest.json` },
  ],
};

describe("the manifest offered to installed copies", () => {
  test("names each package by its download address, signatures unchanged", () => {
    const offered = offerManifest(draft, published, "v0.1.2");
    expect(offered.platforms).toEqual({
      "windows-x86_64": { url: `${DOWNLOAD}/SlopShop_0.1.2_x64-setup.exe`, signature: "sig-nsis" },
      "windows-x86_64-msi": {
        url: `${DOWNLOAD}/SlopShop_0.1.2_x64_en-US.msi`,
        signature: "sig-msi",
      },
      "linux-x86_64": {
        url: `${DOWNLOAD}/SlopShop_0.1.2_amd64.AppImage`,
        signature: "sig-appimage",
      },
    });
    expect(offered.version).toBe("0.1.2");
    expect(offered.pub_date).toBe(draft.pub_date);
  });

  test("takes the notes above the release's first heading", () => {
    expect(offerManifest(draft, published, "v0.1.2").notes).toBe(
      "An early build.\n\n- Faster brushes.",
    );
  });

  test("keeps download addresses of the release as they are", () => {
    const manifest = {
      ...draft,
      platforms: { "linux-x86_64": { url: `${DOWNLOAD}/a.AppImage`, signature: "s" } },
    };
    expect(offerManifest(manifest, published, "v0.1.2").platforms["linux-x86_64"].url).toBe(
      `${DOWNLOAD}/a.AppImage`,
    );
  });

  test("refuses a package of another release, or one the release does not have", () => {
    const other = {
      ...draft,
      platforms: {
        "linux-x86_64": { url: `${DOWNLOAD.replace("v0.1.2", "v0.1.1")}/a`, signature: "s" },
      },
    };
    expect(() => offerManifest(other, published, "v0.1.2")).toThrow(/not a package of v0.1.2/);
    const missing = {
      ...draft,
      platforms: { "linux-x86_64": { url: `${API}/99`, signature: "s" } },
    };
    expect(() => offerManifest(missing, published, "v0.1.2")).toThrow(/not a package/);
    // Still a draft: its addresses do not name the tag yet.
    const unpublished = {
      ...published,
      assets: [{ id: 11, browser_download_url: `${DOWNLOAD.replace("v0.1.2", "untagged-1")}/a` }],
    };
    const one = { ...draft, platforms: { "windows-x86_64": draft.platforms["windows-x86_64"] } };
    expect(() => offerManifest(one, unpublished, "v0.1.2")).toThrow(/not a package/);
  });

  test("refuses the manifest of another version", () => {
    expect(() => offerManifest(draft, published, "v0.1.3")).toThrow(/version 0.1.2, not v0.1.3/);
  });
});

describe("release notes", () => {
  test("a text without headings is kept whole, trimmed", () => {
    expect(releaseNotes("  Fixes.\n")).toBe("Fixes.");
  });

  test("no text, no notes", () => {
    expect(releaseNotes(null)).toBe("");
    expect(releaseNotes("## Which file to download\n")).toBe("");
  });
});
