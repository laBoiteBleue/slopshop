// Localized formatting shared by several components.

import { getLocale } from "./i18n/index.svelte";

/** Zoom (1 = 100%) as a localized percentage, with more decimals when very small. */
export function formatZoom(zoom: number): string {
  const percent = zoom * 100;
  const digits = percent < 1 ? 2 : percent < 10 ? 1 : 0;
  return new Intl.NumberFormat(getLocale(), {
    style: "percent",
    maximumFractionDigits: digits,
  }).format(zoom);
}

/** A size in bytes as a localized amount of MB or GB (1024-based, as Windows shows sizes). */
export function formatBytes(bytes: number): string {
  const mb = bytes / 1024 / 1024;
  const [value, unit] = mb >= 1000 ? [mb / 1024, "gigabyte"] : [mb, "megabyte"];
  return new Intl.NumberFormat(getLocale(), {
    style: "unit",
    unit,
    maximumFractionDigits: value < 10 ? 1 : 0,
  }).format(value);
}
