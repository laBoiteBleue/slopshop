# 0033 — Project license: GPL-3.0-only

Status: accepted (2026-10-04, the maintainer's decision).

## Context

SlopShop was under the MIT license. The maintainer wants it to stay free software
for good: anyone may use it at no cost, commercially included, offline and without an account;
study it, modify it, build their own versions, redistribute them and sell distributions or
services around them. A distributed derived version must keep those freedoms and come with
its source. The project's revenue is not meant to come from restricting the desktop
application.

## Decision

1. **SlopShop's code is licensed under the GNU GPL version 3 only** (`GPL-3.0-only`): every
   crate and the application. `LICENSE` holds the official text; the manifests (`Cargo.toml`
   workspace package, `app/package.json`) declare `GPL-3.0-only`. No version was released
   under MIT; the commits before this change keep their MIT license file in the history.
2. **The dependency policy of [ADR 0006](0006-universal-import-and-licensing.md) is
   unchanged**: dependencies stay permissive, LGPL only isolated, GPL and AGPL refused.
   `deny.toml` exempts SlopShop's own crates only.
3. **The name and the logo are not licensed under the GPL.**
4. Programs outside this repository, such as a hosted service talking to the client over a
   network API, are separate programs under their own license.

Left open, to decide separately:

- **The `.slop` specification** ([file-format.md](../file-format.md)) has no license of its
  own and is covered by the repository's. A separate license, so that other software can
  implement the format, is to decide.
- **Plugins.** No plugin API exists yet (roadmap phase 5). Whether proprietary plugins are
  possible, and how (process boundary, the license of a plugin SDK, an additional permission
  under section 7 of the GPL), is decided with the plugin architecture. Such a permission
  needs the agreement of every copyright holder of the code it covers.

## Alternatives

- **GPL-3.0-or-later**: lets later GPL versions apply; the maintainer chose "only".
- **AGPL-3.0**: also requires the source of versions offered over a network; not wanted.
- **MPL-2.0**: file-level copyleft; a derived version may add closed files.
- **Stay MIT**: allows closed redistributions.

## Consequences

- `deny.toml` allows `GPL-3.0-only` for the `slopshop-*` crates.
- The About dialog and the README state the new license.
- Without contribution terms, outside contributions are expected under GPL-3.0-only
  ("inbound = outbound"); the terms are to settle before the first one is accepted.
