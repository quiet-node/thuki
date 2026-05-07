# Releasing Thuki

Thuki ships signed updates to existing installs through the bundled Tauri updater. This guide walks through cutting a release end to end.

## Prerequisites

- The ed25519 private signing key at `~/.thuki-updater.key` (kept off the repo).
- `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` exported in your shell. The repo convention is to put these in `~/.zshrc.local`:

  ```bash
  export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.thuki-updater.key)"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
  ```

- Push access to `quiet-node/thuki` and `gh` CLI authenticated for that repo.

The matching ed25519 public key is already compiled into the app via `src-tauri/tauri.conf.json`. Existing installs will only accept updates signed with the corresponding private key, so do not lose it. If it is ever rotated, every existing install must be re-published manually.

## Cut a release

1. **Bump the version.** Edit `package.json` and `src-tauri/tauri.conf.json` to the new SemVer string. The two values must match. Commit the bump as `chore(release): vX.Y.Z`.

2. **Update the changelog.** Add a new section to `CHANGELOG.md` (if present) describing user-visible changes. The Tauri manifest links to this for the in-app "Release notes" affordance.

3. **Build signed artifacts.**

   ```bash
   bun run build:all
   ```

   On a successful build, the bundler emits to `src-tauri/target/release/bundle/macos/`:

   - `Thuki.app` (the regular `.app` bundle)
   - `Thuki_<version>_<arch>.app.tar.gz` (the updater payload)
   - `Thuki_<version>_<arch>.app.tar.gz.sig` (the ed25519 signature for the payload)

   Repeat the build on every architecture you intend to ship (`aarch64-apple-darwin`, `x86_64-apple-darwin`).

4. **Create a GitHub release.** Tag it `vX.Y.Z`. Attach the `.app.tar.gz` and the matching `.sig` for each architecture, plus a `latest.json` manifest (template below).

5. **Publish.** GitHub's "latest" alias updates automatically, so existing installs polling the manifest URL will see the new version on their next check.

## `latest.json` template

```json
{
  "version": "0.8.0",
  "notes": "https://github.com/quiet-node/thuki/releases/tag/v0.8.0",
  "pub_date": "2026-05-08T12:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "<contents of Thuki_0.8.0_aarch64.app.tar.gz.sig>",
      "url": "https://github.com/quiet-node/thuki/releases/download/v0.8.0/Thuki_0.8.0_aarch64.app.tar.gz"
    },
    "darwin-x86_64": {
      "signature": "<contents of Thuki_0.8.0_x64.app.tar.gz.sig>",
      "url": "https://github.com/quiet-node/thuki/releases/download/v0.8.0/Thuki_0.8.0_x64.app.tar.gz"
    }
  }
}
```

The `signature` field is the entire content of the matching `.sig` file as a single string. Do not strip whitespace.

## Verify the release

After publishing, fetch the manifest from the latest URL and inspect it:

```bash
curl -sL https://github.com/quiet-node/thuki/releases/latest/download/latest.json | jq .
```

Confirm:

- `version` matches the tag you just pushed.
- `url` for each platform resolves with `curl -I` (HTTP 302 → 200).
- The `signature` for each platform matches the contents of the `.sig` file in the release assets.

For an end-to-end sanity check, install the previous version on a clean macOS account, leave it open for 24 hours (or trigger Settings → Check now), and confirm the in-app banner picks up the new version.

## Rollback

The updater never moves backwards on its own. If a release is bad, publish a higher version that reverts the change.

If a release accidentally ships with an invalid signature, existing installs will reject the payload and surface an "update verification failed" notice. They keep running on their current version. Re-cut the release with a valid signature, increment the patch version, and re-publish.

## Apple Developer Program note

Thuki does not require Apple Developer Program membership. The app is ad-hoc signed at build time. Auto-updates work because Tauri downloads the payload via the application process (no quarantine attribute is set), so Gatekeeper does not block the swapped binary at relaunch. First-install Gatekeeper friction (right-click, Open) still applies for users downloading the `.app` directly from a release page.
