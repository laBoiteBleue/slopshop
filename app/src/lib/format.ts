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
