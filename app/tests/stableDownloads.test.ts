import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { INSTALLERS, stableCopies } from "../scripts/stable-downloads.mjs";

/** The files of a release, as release.yml attaches them (v0.1.1). */
const release = (version: string) => [
  "latest.json",
  `SlopShop-${version}-1.x86_64.rpm`,
  `SlopShop-${version}-1.x86_64.rpm.sig`,
  `SlopShop_${version}_aarch64.app.tar.gz`,
  `SlopShop_${version}_aarch64.app.tar.gz.sig`,
  `SlopShop_${version}_aarch64.dmg`,
  `SlopShop_${version}_amd64.AppImage`,
  `SlopShop_${version}_amd64.AppImage.sig`,
  `SlopShop_${version}_amd64.deb`,
  `SlopShop_${version}_amd64.deb.sig`,
  `SlopShop_${version}_x64-setup.exe`,
  `SlopShop_${version}_x64-setup.exe.sig`,
  `SlopShop_${version}_x64.app.tar.gz`,
  `SlopShop_${version}_x64.app.tar.gz.sig`,
  `SlopShop_${version}_x64.dmg`,
  `SlopShop_${version}_x64_en-US.msi`,
  `SlopShop_${version}_x64_en-US.msi.sig`,
];

test("each installer of the release gets a copy without the version", () => {
  expect(stableCopies(release("0.1.2"), "0.1.2")).toEqual([
    ["SlopShop_0.1.2_x64-setup.exe", "SlopShop_x64-setup.exe"],
    ["SlopShop_0.1.2_x64_en-US.msi", "SlopShop_x64_en-US.msi"],
    ["SlopShop_0.1.2_aarch64.dmg", "SlopShop_aarch64.dmg"],
    ["SlopShop_0.1.2_x64.dmg", "SlopShop_x64.dmg"],
    ["SlopShop_0.1.2_amd64.deb", "SlopShop_amd64.deb"],
    ["SlopShop-0.1.2-1.x86_64.rpm", "SlopShop.x86_64.rpm"],
    ["SlopShop_0.1.2_amd64.AppImage", "SlopShop_amd64.AppImage"],
  ]);
});

test("a missing installer stops the copy", () => {
  const names = release("0.1.2").filter((name) => !name.endsWith(".dmg"));
  expect(() => stableCopies(names, "0.1.2")).toThrow("SlopShop_0.1.2_aarch64.dmg");
  // Another version's files are not this one's.
  expect(() => stableCopies(release("0.1.1"), "0.1.2")).toThrow("is not attached");
});

test("the README links to the copies, and only to them", () => {
  const readme = readFileSync(resolve(__dirname, "../../README.md"), "utf8");
  const linked = [...readme.matchAll(/releases\/latest\/download\/([^)\s]+)\)/g)].map((m) => m[1]);
  const copies = INSTALLERS.map(([, stable]) => stable);
  // In the French and the English part.
  expect(new Set(linked)).toEqual(new Set(copies));
  expect(linked).toHaveLength(2 * copies.length);
});
