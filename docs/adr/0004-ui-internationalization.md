# 0004 — UI internationalization

Status: accepted

## Context

The repository and code are in English, but the application must be available in French and
English from the start, and adding languages must be easy.

## Decision

- A small in-house i18n module in `app/src/lib/i18n/` (no dependency):
  - `en.ts` is the reference catalog (flat keys, `{param}` placeholders);
  - other catalogs are typed as `Messages`, so a missing key fails `npm run check`;
  - `locales` registers catalogs; adding a language is one file plus one registry entry;
  - `t(key, params)` is reactive (Svelte runes), numbers are formatted per locale;
  - the locale is detected from the system, can be changed in the UI and is remembered.
- The engine and IPC transmit identifiers and structured data, never display text.

## Alternatives

- **Paraglide / svelte-i18n / i18next**: richer (ICU plurals, lazy loading, tooling), but more
  dependencies and build steps than the current handful of strings justify. Revisit when
  plurals, many languages or translator tooling are needed — the `t()` call sites can stay.

## Consequences

- No ICU pluralization yet; use `Intl.PluralRules` if a message needs it.
- Engine errors are still English strings; user-facing errors will need codes + parameters.
