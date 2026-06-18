# Releasing Thuki

Thuki ships signed updates to existing installs through the bundled Tauri updater. Releases are fully automated: the GitHub Actions workflow builds, signs, and publishes everything when a release-please PR merges.

## Day-to-day: nothing to do

Releases happen automatically. Land conventional-commit PRs into `main`. release-please opens a release PR. Merging that PR cuts a tag, which triggers the build workflow. The workflow produces:

- `Thuki.dmg` (fresh-install download)
- `Thuki_<version>_aarch64.app.tar.gz` (updater payload, ad-hoc-signed `.app` inside)
- `Thuki_<version>_aarch64.app.tar.gz.sig` (ed25519 signature for the payload)
- `latest.json` (the manifest the in-app updater polls)

All four are uploaded to the GitHub release. Existing v0.7.x installs detect the new version on their next 24-hour check and offer to install in place.

## Where the signing key lives

The ed25519 private key is stored in **GitHub Actions secrets**, not on any developer laptop:

- `TAURI_SIGNING_PRIVATE_KEY`: contents of the private key file.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: empty for the current key, kept as a secret for future password-protected rotations.

The matching public key is committed to the repo at `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. Every Thuki binary verifies updates against that public key. An attacker who replaces a release file cannot also forge a valid signature without the private key, so the swap is rejected and the running app keeps its current version.

A backup copy of both keys lives in the private `quiet-node/thuki-confidential` repo. That copy is the disaster-recovery anchor: if GitHub Actions secrets ever get wiped, restore from the backup; if the backup is ever compromised, rotate the keypair (which orphans every existing install at its current version, so do this only as a last resort).

## Local development: no keys required

`bun run build:all` and `bun run validate-build` produce an unsigned `.app` bundle. Devs can launch it, test production behavior, and verify everything compiles. The signing step is gated behind `bun run build:release`, which is only invoked by CI.

There is nothing to set up on your laptop. No env vars, no key files, no `.zshrc.local` overrides. New contributors clone the repo and start working.

## Bundled inference engine

Every build embeds llama.cpp's `llama-server` as a Tauri sidecar. The binary and the dylibs it links are fetched and verified by `scripts/ensure-llama-server.ts`, which pins an exact llama.cpp release tag and the sha256 of its macOS arm64 asset; a hash mismatch aborts the build. The script runs automatically in front of `dev`, `build:backend`, and `build:release`, and is an instant no-op once the pinned version is installed under `src-tauri/binaries/` (gitignored, never committed). CI caches that directory with a key derived from the pinned version and hash, so release builds only hit GitHub's release CDN when the pin changes. Because the script adds an `@loader_path/../Frameworks` rpath for bundle-time dylib resolution, it ad-hoc re-signs the binary and each dylib after the edit.

Developer ID signing and notarization are a release-time prerequisite for shipping without the Gatekeeper quarantine workaround; they land as a release workflow step once the Apple Developer certificate exists. Caveat for that step: the sidecar's dylibs live nested under `Contents/Frameworks/`, and a plain `codesign` of the `.app` does not re-sign them, so the workflow must deep-sign the nested dylibs (each dylib and the `llama-server` binary individually, innermost first) before notarization or Apple's service rejects the bundle.

### Bumping the pinned llama.cpp version

The pin in `scripts/ensure-llama-server.ts` is two constants. `LLAMA_CPP_TAG` names a published llama.cpp release (for example `b9590`, listed at https://github.com/ggml-org/llama.cpp/releases), and `ASSET_SHA256` is the sha256 of that release's `llama-<tag>-bin-macos-arm64.tar.gz` asset. This is a release pin, not a git commit: llama.cpp's `main` branch moving forward does not affect a pinned build, and a newer release does not make the current one stop working. The pin is updated only when we deliberately adopt a newer engine.

There is no automatic bump, and that is intentional: a new engine version has to clear the manual checks below on real hardware before it ships. Upgrade when there is a concrete reason: a newer model architecture we want to load, a `llama-server` bug or security fix, or a Metal/performance improvement. Otherwise the existing pin keeps working indefinitely.

To bump:

1. Pick the target release on https://github.com/ggml-org/llama.cpp/releases and set `LLAMA_CPP_TAG` to its tag.
2. Set `ASSET_SHA256` to the macOS arm64 asset's hash. Read it from the GitHub Releases API (the asset's `digest` field) or compute it locally with `shasum -a 256 llama-<tag>-bin-macos-arm64.tar.gz`.
3. Run `bun run engine:ensure`. It fetches the new asset, verifies the new hash, and re-derives the dylib link closure. If the new release adds, renames, or drops a dylib, the script aborts and names exactly which entries differ from `bundle.macOS.frameworks` in `src-tauri/tauri.conf.json`; update that list to match so the closure check passes.
4. Bump the cache key in the build workflows so the new asset is not served stale from the old cache.
5. Re-run the binary-dependent checks on a real machine: the sidecar spawns and streams a response, and `codesign -vv` is clean on the `llama-server` binary and every bundled dylib.

## Cutting a release manually (rare)

If for some reason a release must be cut outside of CI (incident response, rolling back a bad release-please commit, etc.), the procedure is:

1. Restore the keypair from `quiet-node/thuki-confidential` to a temporary location.
2. Export the env vars in the shell that runs the build:

   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="$(cat /path/to/restored/thuki-updater.key)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   ```

3. Bump versions in `package.json` and `src-tauri/tauri.conf.json` to match.
4. Build the signed payload:

   ```bash
   bun run build:release
   ```

5. Codesign the inner `.app` with `codesign --deep --force --sign - <Thuki.app>`.
6. Hand-craft `latest.json` (see template below) and upload it alongside the `.tar.gz`, `.sig`, and `Thuki.dmg` to the GitHub release.
7. Securely delete the restored key from the temporary location.

This path is documented for completeness only. CI is the supported path.

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
    }
  }
}
```

The `signature` field is the entire content of the matching `.sig` file as a single string. Do not strip whitespace.

## Verify a release

After a release publishes, fetch the manifest:

```bash
curl -sL https://github.com/quiet-node/thuki/releases/latest/download/latest.json | jq .
```

Check that `version` matches the new tag, `url` resolves, and `signature` matches the contents of the `.sig` file in the release assets.

For an end-to-end smoke test, install the previous version on a clean macOS account, leave it open for 24 hours (or trigger Settings → Check now), and confirm the in-app banner picks up the new version and installs cleanly.

## Rollback

The updater never moves backwards on its own. If a release is bad, publish a higher version that reverts the change.

If a release ships with an invalid signature, existing installs reject the payload and surface an "update verification failed" message. They keep running on their current version. Re-cut the release with a valid signature, increment the patch version, and re-publish.

## Apple Developer Program note

Thuki does not require Apple Developer Program membership. The app is ad-hoc signed at build time. Auto-updates work because the Tauri updater downloads the payload via the application process, so no quarantine attribute is set on the swapped binary and Gatekeeper does not re-prompt at relaunch. First-install Gatekeeper friction (right-click, Open) still applies for users downloading the `.app` directly from a release page.
