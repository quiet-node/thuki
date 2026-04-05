# Releasing Thuki

Releases are fully automated. There is no manual tagging, no manual changelog editing, and no manual version bumping.

## How It Works

Every commit merged to `main` that follows the [Conventional Commits](https://www.conventionalcommits.org/) format is picked up by [release-please](https://github.com/googleapis/release-please). It continuously maintains an open "release PR" that:

- Bumps the version in `package.json` and `src-tauri/Cargo.toml`
- Updates `CHANGELOG.md` with the new entries grouped by type

When you are ready to ship, merge that PR. That triggers the build workflow, which:

1. Runs the full test and validation suite
2. Builds `Thuki.app`
3. Creates a GitHub Release with the changelog notes and `Thuki.app.tar.gz` as the downloadable asset

## Commit Types and Version Bumps

release-please reads commit prefixes to decide how to bump the version:

| Commit prefix | Example | Version bump |
| :--- | :--- | :--- |
| `feat:` | `feat: add voice input` | Minor (`0.1.0` → `0.2.0`) |
| `fix:` | `fix: hotkey not firing` | Patch (`0.1.0` → `0.1.1`) |
| `feat!:` or `BREAKING CHANGE:` | `feat!: new IPC protocol` | Major (`0.1.0` → `1.0.0`) |
| `docs:`, `chore:`, `refactor:`, etc. | `docs: update README` | No bump |

## Step-by-Step

1. Merge feature and fix PRs to `main` as normal, using conventional commit messages.
2. release-please automatically opens or updates a release PR titled `chore(main): release X.Y.Z`.
3. Review the PR to confirm the version and changelog look right.
4. Merge the release PR.
5. Done. The build runs automatically and the GitHub Release is published.

## Files Managed by release-please

Do not edit these manually:

- `CHANGELOG.md`: written by release-please on every release PR
- `package.json` `"version"` field: bumped automatically
- `src-tauri/Cargo.toml` `version` field: bumped automatically
- `.release-please-manifest.json`: tracks the current released version; do not edit
