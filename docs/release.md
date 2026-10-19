# Release versioning

Concept for [#27](https://github.com/arstulke/aw-manual/issues/27): the
installers of v0.2.0 and v0.2.1 carry version `0.1.0`. This page explains why,
what changed, and how the release process now guards against the same mistake.

## What went wrong

Before this change the version was written in four places, and a release had to
update all of them by hand:

| File                            | Used for                                            | v0.2.0  | v0.2.1  |
| ------------------------------- | --------------------------------------------------- | ------- | ------- |
| `Cargo.toml`                    | crate versions, `--version`, user agent, Nix package | `0.1.0` | `0.2.1` |
| `crates/daemon/tauri.conf.json` | installer names and package metadata                 | `0.1.0` | `0.1.0` |
| `ui/package.json`               | nothing                                              | `0.1.0` | `0.1.0` |
| `ui/package-lock.json`          | nothing                                              | `0.1.0` | `0.1.0` |

- **v0.2.1:** `Cargo.toml` was bumped, `tauri.conf.json` wasn't. The binaries
  reported 0.2.1, but Tauri named and versioned the deb, dmg, msi and exe
  `0.1.0`, because an explicit `version` in `tauri.conf.json` takes precedence
  over `Cargo.toml`.
- **v0.2.0:** the tag was pushed without any version bump. Even the binaries of
  that release report `0.1.0`.

Two separate human errors, and nothing in CI noticed either of them.

## Principles

1. **One source of truth.** The version is written in exactly one place:
   `workspace.package.version` in the root `Cargo.toml`. Everything else
   derives it from there.
2. **The tag follows the code, not the other way around.** Nobody creates
   release tags. Merging a new version to `main` is the release: CI builds it,
   creates the `v<version>` tag on that commit and the draft release.
3. **CI verifies the result, not just the inputs.** Even with a single source
   of truth, CI checks the built installers, so a regression in the tooling (for
   example a Tauri update that changes the fallback) is caught as well.
4. **Fail before publishing.** Every check runs before the tag and the draft
   release are created.

## Single source of truth

| Consumer                          | How it gets the version                                                               |
| --------------------------------- | ------------------------------------------------------------------------------------- |
| `aw-manual-core`, `-cli`, `-daemon` | `version.workspace = true`                                                          |
| Binaries (`--version`, user agent, settings page, IPC `status`) | `env!("CARGO_PKG_VERSION")`                       |
| Tauri bundles (deb, dmg, msi, exe) | `tauri.conf.json` has no `version`; Tauri then reads it from the daemon's `Cargo.toml`, resolving `version.workspace = true` |
| Nix package                       | `(lib.importTOML ../Cargo.toml).workspace.package.version`                            |
| `ui/package.json`                 | no `version` (the package is private and never published)                            |
| `Cargo.lock`                      | updated by `scripts/version.mjs bump` (or any `cargo` command)                        |

`scripts/version.mjs check` fails if any of these files declare a version of
their own again, if a crate stops inheriting the workspace version, or if
`Cargo.lock` is out of date.

Alternatives considered:

- *Derive the version from the git tag in CI* (patch the files at build time).
  Rejected: local builds, `nix build` and `cargo install` would no longer know
  the version, and the tree would never say which version it is.
- *Keep the copies and sync them with a script.* Rejected: every copy is a
  chance to forget one; removing the copies removes the failure mode.

## Release process

```sh
node scripts/version.mjs bump 0.3.0     # Cargo.toml + Cargo.lock
git commit -am "chore(release): v0.3.0" # open a PR and merge it to main
```

That's all. The push to `main` runs the pipeline:

1. The `Version` job checks the single source of truth and runs
   `version.mjs release-info`: a version counts as released when its
   `v<version>` tag exists. If it doesn't, the push releases it.
2. Tests and bundles run as on every push.
3. The `GitHub release` job (only on `main`, only for an unreleased version)
   checks the installers, creates the annotated tag `v<version>` on the built
   commit and a **draft** GitHub release with the deb, dmg, msi and exe
   packages. Add release notes and publish it.

Pushes to `main` that don't change the version release nothing. A tag check is
used instead of diffing `Cargo.toml` against the previous commit: if a release
run fails (for example a flaky bundle job), the next push to `main`, or a re-run,
still releases the version instead of skipping it. The tag step never moves an
existing tag; re-running it on the same commit is a no-op.

The pipeline no longer runs for pushed tags, so a tag created by hand builds
and releases nothing.

`bump` refuses versions that are not `x.y.z` (MSI only accepts numeric
versions) or not newer than the current one. `release-info` fails (on PRs as
well) if an unreleased version is not newer than the latest release, so a typo
like `0.1.5` after `0.2.1` is caught before merging.

## Safety net in CI

| Where                         | Check                                                                               | Catches                                 |
| ----------------------------- | ----------------------------------------------------------------------------------- | --------------------------------------- |
| `Version` job (every push/PR) | `version.mjs check`: single source of truth, lock file in sync                       | a second version reappearing (v0.2.1)   |
| `Version` job (every push/PR) | `version.mjs release-info`: an unreleased version is newer than the latest release   | version going backwards                 |
| `Bundle` jobs                 | `version.mjs assets`: every installer name carries `_<version>_`; the deb's control `Version` matches | tooling regressions, wrong metadata |
| `GitHub release` job          | both checks again on the downloaded artifacts, before tagging                       | anything that slipped through           |

The v0.2.0 mistake (a tag without a bump) can no longer happen: tags are only
created by CI, from the version in the tree.

Optional hardening in the repository settings:

- Branch protection on `main` requiring the `Version` check.
- A tag ruleset for `v*` that blocks updating or deleting tags. If it also
  restricts creation, allow GitHub Actions to bypass it, since the pipeline
  creates the tags.

## Fixing v0.2.0 and v0.2.1

The binaries of v0.2.1 are correct; only the installer names and metadata are
not. v0.2.0 is wrong throughout. Both can't be fixed in place without moving
the tags, which would make the same tag point to different code. Options:

1. **Recommended:** release **v0.2.2** (`version.mjs bump 0.2.2`) with this fix, and add a
   note to the v0.2.0 and v0.2.1 release descriptions that their installers
   report `0.1.0` and should be replaced with v0.2.2. Upgrades work, since
   0.2.2 is newer than both 0.1.0 and 0.2.1.
2. Additionally remove the misnamed assets from v0.2.0 and v0.2.1, so nobody
   installs a package that claims to be 0.1.0.
