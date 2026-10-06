# 0039 — In-app updates

Status: accepted (2026-10-06; the maintainer chose a full in-app update, a startup check that
can be turned off, an address that works with pre-releases and a discreet notice).

## Context

SlopShop 0.1.0 shipped as installers on GitHub releases, with no way to learn of a new version
from the application. Users would stay on old versions, with their bugs. Tauri has an official
updater plugin (`tauri-plugin-updater`, MIT OR Apache-2.0): it reads a JSON manifest, downloads
the package for the running system, checks its signature against a public key built into the
application, and installs it. It updates the Windows installers (NSIS or MSI), the macOS
`.app` (from a `.app.tar.gz`) and the Linux AppImage; `.deb` and `.rpm` stay with the system's
package manager.

Releases are built as drafts, tested by the maintainer on Windows, then published; they are
marked pre-release, which GitHub's `releases/latest` address ignores. (Since 0.1.2 they are
normal releases, so that the README can link to the latest one; the manifest stays on the
`updates` pre-release, which installed copies already read.)

## Decision

1. **Full update from the application**, with `tauri-plugin-updater` driven from Rust
   (`app/src-tauri/src/update.rs`): check, download with progress over a `Channel` (as the AI
   components' download), install, restart (`AppHandle::restart`, no process plugin). The web
   view gets no updater permission; the UI only calls the app's commands.
2. **Signed packages.** The maintainer holds the signing key (a minisign pair from
   `tauri signer generate`); its public half is in the bundle configuration, its private half
   and password in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. This is not code signing: Windows and macOS still warn
   about unsigned installers. Losing the key means installed copies can no longer be updated.
3. **The manifest moves only when a release is published.** The release workflow builds the
   update packages and `latest.json` into the draft, with download addresses that name the tag
   (`releases/download/vX.Y.Z/…`). When the maintainer publishes it, a workflow copies its
   `latest.json` to a permanent pre-release tagged `updates`, which holds nothing else; the
   application reads `releases/download/updates/latest.json`. Drafts and tags reach no one.
   On Windows the manifest points to the NSIS installer (`setup.exe`, a per-user install that
   needs no administrator rights), installed in passive mode (a progress bar, no questions).
4. **When the application checks**: a quiet check after startup, at most once a day, which
   Edit > Preferences > Updates can turn off; and Help > Check for Updates, always available.
   A failed automatic check says nothing; a failed manual one says why. The check sends GitHub
   what any download does (address, user agent), nothing else.
5. **A discreet notice**: when a version is found, a button appears at the right of the menu
   bar; it opens the update dialog (the new version, its notes, Install and Restart / Later).
   Nothing opens by itself.
6. **Nothing is lost by installing.** Before installing, unsaved documents are offered for
   saving as when quitting, and the install is refused while exports run.
7. **Only builds that can update offer it.** The updater is configured in
   `tauri.bundle.conf.json`, used for releases; development builds, builds from source and
   Linux `.deb`/`.rpm` installs have no check, no menu entry and no preference.

## Alternatives

- **Announce only** (link to the release page, no key, no dependency): simpler, but each update
  is a manual download and install, and few users would do it.
- **Read `releases/latest`**: requires releases that are not marked pre-release; and tauri-action
  writes manifest addresses that follow `latest`, so a newer pre-release would never be offered.
- **Check only on request**: no network access without an action, but most users would never
  look; the preference covers those who want that.
- **A dialog at startup**: more visible, but it interrupts work.
- **Driving the plugin from JavaScript**: less Rust, but it grants the web view the updater's
  permissions and spreads the flow between the UI and the shell.

## Consequences

- Two new dependencies in the app shell: the updater plugin and what it brings for HTTPS
  downloads (checked by `cargo deny`).
- Copies of 0.1.0 have no updater: their users install the first version with one by hand.
- The macOS `.app.tar.gz` archives become part of each release (they were deleted by hand).
- Distributions that package SlopShop update it themselves; they build without the bundle
  configuration and get no updater.
- The maintainer tests on Windows only: updates on macOS and Linux (AppImage) rely on CI and are
  untried.
