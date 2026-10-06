## What and why

<!-- What this pull request changes, and the issue it addresses (Closes #…). -->

## How it was tested

<!-- Which tests cover the change (Rust and UI), the steps in the app, and on which platform
(Windows, macOS, Linux). -->

## Checklist

- [ ] Every commit is signed off (`git commit -s`), certifying the
      [Developer Certificate of Origin](https://github.com/laBoiteBleue/slopshop/blob/main/DCO)
- [ ] Tests cover the change: unit, integration, round trip or damaged input as relevant, UI
      component tests for a changed control, and a regression test for a bug fix
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace` and `cargo deny check` pass
- [ ] `npm run format`, `npm run check` and `npm test` pass in `app/` (when the UI changed)
- [ ] User-visible strings go through i18n and every catalog is updated
- [ ] Docs updated where relevant (README, `docs/formats.md`, `docs/cli.md`,
      `docs/feature-map.csv`, roadmap)
- [ ] An AI tool helped: yes / no (if yes: I have read and understood every line, and the tests
      above are what proves it works)
