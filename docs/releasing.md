# Releasing SlopShop

How the maintainer publishes a new version. The installers are built by CI
(`.github/workflows/release.yml`); installed copies learn of the version once it is published
(`.github/workflows/updates.yml`, ADR 0039).

## 1. Set the version

The version is written in one place: `version` under `[workspace.package]` in the root
`Cargo.toml` (a test fails if another one appears). Change it, then update the lock file, which
CI checks:

```sh
cargo update --workspace
```

Commit both files in a pull request (`chore: version 0.1.1`) and merge it.

## 2. Tag it

On an up-to-date `main`:

```sh
git checkout main
git pull
git tag v0.1.1
git push origin v0.1.1
```

The tag must be the version with a `v` in front: the Release workflow stops otherwise. It
builds the installers for Windows, macOS and Linux, and the update packages signed with the
maintainer's key (repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`), and attaches them to a **draft** pre-release with the
notes of `.github/release-notes.md`. It takes about 25 minutes.

## 3. Check the draft

In the repository's Releases, open the draft: install the Windows installer and try it. Add
what changed since the previous version at the top of the notes. Keep every file attached,
including `latest.json`, the `.sig` files and the macOS `.app.tar.gz` archives: they are the
update packages.

## 4. Publish

Click **Publish release**. The Updates workflow copies the release's `latest.json` to the
`updates` pre-release, which installed copies read: from then on they are offered the new
version (at their next startup check, at most once a day, or with Help > Check for Updates).
Drafts and tags never reach installed copies.

To offer a release again (for example after replacing its `latest.json`), run the Updates
workflow by hand with its tag. Installed copies never go back to an older version.

## The signing key

The update packages are signed with a minisign key the maintainer keeps outside the repository,
with its password, and backs up. Without it no installed copy can be updated again; it is not
the code signing that Windows and macOS warn about.
