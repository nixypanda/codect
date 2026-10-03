# Releasing

Releases are cut from `main` by pushing a `vX.Y.Z` tag. Continuous integration
runs on pull requests and `main`; the
[release workflow](../.github/workflows/release.yml) only verifies the tag,
builds the four supported targets, and publishes a GitHub Release.

## Version source of truth

The version appears in exactly two committed places, and the workflow fails if
the tag disagrees with either:

| File | Line |
| --- | --- |
| `Cargo.toml` | `[workspace.package] version` |
| `flake.nix` | the `version` binding near the top of the `outputs` `let` |

Individual crates inherit the workspace version (`version.workspace = true`), so
only the one `Cargo.toml` line changes. The flake binding is shared by
`packages.codect` and `packages.codect-nvim`.

## Steps

1. Bump the version in both files above.
2. Refresh the committed lockfile so the local-crate entries match:

   ```sh
   cargo build --workspace
   ```

3. Commit the bump:

   ```sh
   git commit -am "release: vX.Y.Z"
   ```

4. Tag and push:

   ```sh
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

The tag push starts the release workflow.

## What the workflow publishes

Four archives, each containing the `codect` binary, `LICENSE`, and `README.md`:

| Target | Runner |
| --- | --- |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| `aarch64-apple-darwin` | `macos-latest` |
| `x86_64-apple-darwin` | `macos-latest` (cross-compiled) |

Alongside them: a `SHA256SUMS` file and a signed build-provenance attestation
for every archive.

Nix installs (`nix profile install github:nixypanda/codect`) pick up the tag
automatically; the tarballs are built with cargo, not Nix, so they stay portable
to machines without a Nix store.

## Verifying a release

```sh
sha256sum -c SHA256SUMS        # or: shasum -a 256 -c SHA256SUMS

gh attestation verify codect-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz \
  --repo nixypanda/codect
```
