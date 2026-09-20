# Any-Cal packaging

The release script produces a relocatable user-local stage for the headless
`any-cal` service and the optional `any-cal-gui` binary. It builds offline,
stages documentation and a token-free configuration example, then writes
`MANIFEST` and `SHA256SUMS`.

```sh
mkdir -p ./tmp
packaging/release.sh --stage ./tmp/any-cal-stage
ANY_CAL_PREFIX="$HOME/.local" packaging/install.sh ./tmp/any-cal-stage
packaging/uninstall.sh
```

Install and upgrade do not need root and never remove configuration or Anytype
data. The GUI contains no separate service logic. Signing, notarization,
Flatpak, MSIX, and native desktop bundles are deferred until release ownership
and signing identities exist.

The scripts are POSIX-shell workflows tested for Linux and intended for macOS
with `bash`, `install`, `find`, `awk`, and either `sha256sum` or `shasum`.
Native Windows packaging, cross-compilation, and CI publication are deferred;
no cross-platform binary claim is made. `--skip-build` is a developer-only
mode and requires the release-version marker created by a normal build.

Hosted CI first warms the Cargo registry (the workflow runs `cargo fetch`);
the subsequent verification and release steps use locked offline Cargo
commands. Local packaging tests keep their temporary stage below `./tmp`.
