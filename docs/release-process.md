# Releasing Thuki

Thuki ships signed updates to existing installs through the bundled Tauri updater. Releases are fully automated: the GitHub Actions workflow builds, signs, and publishes everything when a release-please PR merges.

## Day-to-day: nothing to do

Releases happen automatically. Land conventional-commit PRs into `main`. release-please opens a release PR. Merging that PR cuts a tag, which triggers the build workflow. The workflow produces:

- `Thuki.dmg` (fresh-install download)
- `Thuki_<version>_aarch64.app.tar.gz` (updater payload, ad-hoc-signed `.app` inside)
- `Thuki_<version>_aarch64.app.tar.gz.sig` (ed25519 signature for the payload)
- `latest.json` (the manifest the in-app updater polls)

All four are uploaded to the GitHub release. Existing installs detect the new version on their next 24-hour check and offer to install in place.

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

Every build embeds llama.cpp's `llama-server` as a Tauri sidecar, built from source and verified by `scripts/ensure-llama-server.ts` before `dev`, `build:backend`, and `build:release`. How the pin works, what the script does, and where the files land are documented in [Models and Providers → How the engine binary is packaged](./models-and-providers.md#how-the-engine-binary-is-packaged). The release-specific fact: CI caches `src-tauri/binaries/` with a key derived from the pinned tag and commit, so the engine is compiled only when the pin changes; every other build restores it from cache.

Developer ID signing and notarization are a release-time prerequisite for shipping without the Gatekeeper quarantine workaround; they land as a release workflow step once the Apple Developer certificate exists. Caveat for that step: the sidecar's dylibs live nested under `Contents/Frameworks/`, and a plain `codesign` of the `.app` does not re-sign them, so the workflow must deep-sign the nested dylibs (each dylib and the `llama-server` binary individually, innermost first) before notarization or Apple's service rejects the bundle.

### Bumping the pinned llama.cpp version

The pin is two constants in `scripts/ensure-llama-server.ts`: `LLAMA_CPP_TAG` (the release tag) and `LLAMA_CPP_COMMIT` (that tag's exact commit SHA). The script clones the tag and refuses to build unless `HEAD` matches `LLAMA_CPP_COMMIT`, so the commit is the supply-chain anchor: a moved or forged tag is rejected before anything is built. We build from source rather than download ggml-org's prebuilt because that prebuilt is compiled for macOS 26+ and dyld-fails on older macOS; our build pins `CMAKE_OSX_DEPLOYMENT_TARGET=13.4`, which makes the macOS-15 Metal residency-set symbol a weak import (used on 15+, skipped below). See [Models and Providers](./models-and-providers.md#how-the-engine-binary-is-packaged) for the full why.

There is no automatic bump, and that is intentional: a new engine version has to clear the manual checks below on real hardware before it ships. Upgrade when there is a concrete reason: a newer model architecture we want to load, a `llama-server` bug or security fix, or a Metal/performance improvement. Otherwise the existing pin keeps working indefinitely.

To bump:

1. Pick the target release on https://github.com/ggml-org/llama.cpp/releases and set `LLAMA_CPP_TAG` to its tag.
2. Set `LLAMA_CPP_COMMIT` to that tag's commit SHA: `git ls-remote https://github.com/ggml-org/llama.cpp refs/tags/<tag>`.
3. Run `bun run engine:ensure`. It clones the pinned commit, builds `llama-server` for the macOS 13.4 target, audits the output (arm64, macOS-13.4 deployment target, weak Metal residency-set import, no non-system dylibs), and re-derives the dylib link closure. If the new release adds, renames, or drops a dylib, the script aborts and names exactly which entries differ from `bundle.macOS.frameworks` in `src-tauri/tauri.conf.json`; update that list to match so the closure check passes.
4. Bump the cache-key commit suffix (`-<commit-prefix>`) in the build workflows so the engine is rebuilt rather than restored stale from the old cache.
5. Re-run the binary-dependent checks on a real machine, ideally one running the minimum macOS (13.4), since that compatibility is the whole point of building from source: the sidecar spawns and streams a response, and `codesign -vv` is clean on the `llama-server` binary and every bundled dylib.

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
