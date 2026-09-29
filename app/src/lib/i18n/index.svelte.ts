// Minimal, dependency-free i18n.
//
// To add a language: create `<code>.ts` exporting a `Messages` object (the compiler lists any
// missing key), then register it in `locales` below. Nothing else changes.

import en, { type MessageKey, type Messages } from "./en";
import fr from "./fr";

export const locales = {
  en: { name: "English", messages: en },
  fr: { name: "Français", messages: fr },
} satisfies Record<string, { name: string; messages: Messages }>;

export type Locale = keyof typeof locales;

const STORAGE_KEY = "slopshop.locale";

function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && value in locales;
}

/** Saved choice, else the first system language we support, else English. */
function initialLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (isLocale(saved)) return saved;
  } catch {
    // Storage unavailable: fall back to detection.
  }
  for (const tag of navigator.languages ?? [navigator.language]) {
    const base = tag.toLowerCase().split("-")[0];
    if (isLocale(base)) return base;
  }
  return "en";
}

const initial = initialLocale();
let current = $state<Locale>(initial);
document.documentElement.lang = initial;

export function getLocale(): Locale {
  return current;
}

export function setLocale(locale: Locale) {
  current = locale;
  document.documentElement.lang = locale;
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // Not persisted; the choice still applies to this session.
  }
}

/**
 * Translate `key` in the current locale, replacing `{param}` placeholders. Reactive: reading
 * it in a template re-renders when the locale changes. Numbers are formatted for the locale.
 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const message = locales[current].messages[key] ?? en[key];
  if (!params) return message;
  return message.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    if (value === undefined) return match;
    return typeof value === "number" ? value.toLocaleString(current) : value;
  });
}
