# Contributing to Vertep

Issues and feedback are the most useful contribution right now; pull requests are welcome too.
This page is about building and checking the project from a clean checkout. The design lives in
[`tg.md/`](tg.md) (a typed graph, read and checked with the `tg` CLI) and the requirements in
[`openspec/`](openspec); [CLAUDE.md](CLAUDE.md) lists the conventions both follow.

## The Rust workspace

You need [rustup](https://rustup.rs). The first `cargo` command inside the repository installs the
toolchain named in [`rust-toolchain.toml`](rust-toolchain.toml) with its components and targets.

```sh
cargo build --workspace
cargo test --workspace            # what CI runs on every push
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

The Lean model builds separately: `lean/vertep-lean/scripts/check.sh` (see
[site/docs/verification.html](site/docs/verification.html)).

## The one-command build: @vertep/web and the Python wheel

```sh
scripts/build.sh --container      # needs only Docker (or CONTAINER=podman)
scripts/build.sh                  # the same on this machine, with the toolchain installed
```

Either builds `@vertep/web`, `@vertep/dm-ecma` and the `vertep` wheel from a clean checkout,
smoke-tests each in a fresh project or virtual environment, and leaves them in `dist/`:

```text
dist/npm/vertep-web-<version>.tgz
dist/npm/vertep-dm-ecma-<version>.tgz
dist/python/vertep-<version>-cp39-abi3-<platform>.whl
```

`--only npm` or `--only wheel` builds one of the two. CI runs `scripts/build.sh --container` on
every push (job `reproducible-build`), so a step that only works on one machine fails there.

**The pins.** Every tool version is in [`scripts/toolchain.env`](scripts/toolchain.env): Rust,
the `wasm-bindgen` CLI (which must equal the `wasm-bindgen` library in `Cargo.lock` exactly),
Node.js, LLVM and maturin, with the checksums the image verifies. The image is
[`.devcontainer/Dockerfile`](.devcontainer/Dockerfile) (the Rust image by digest); VS Code and
Codespaces open the repository in it. `scripts/check-toolchain.sh` fails, naming both versions,
when `rust-toolchain.toml`, the Dockerfile, `crates/vertep-wasm/Cargo.toml` or `Cargo.lock`
disagrees with the pins. To move a pin, change `scripts/toolchain.env`, run the check and fix what
it names.

**Without the container**, `scripts/build.sh` needs rustup, Node.js 22 or later, Python 3.9 or
later, and a clang and llvm-ar that target wasm32 (the journal build compiles SQLite's C sources):

- Debian or Ubuntu: `apt install clang llvm`.
- macOS: `brew install llvm`. Apple's clang cannot compile for wasm32, so the script uses
  Homebrew's. If `brew upgrade z3` has removed the `libz3` version that LLVM was linked against,
  clang aborts with `dyld: Library not loaded: .../libz3.4.15.dylib`; the script detects that and
  builds through wrappers under `target/toolchain/` that resolve the old name to the installed
  library. `brew reinstall llvm` fixes it for good.

A missing or mismatched `wasm-bindgen` CLI is installed at the pinned version into
`target/tools/`, and maturin likewise; nothing is installed globally.

For day-to-day work on the JavaScript packages, `npm run build` and `npm test` in
[`bindings/js`](bindings/js) build every variant (including the test-only module and the native
helper the tests compare with); see [site/docs/browser.html](site/docs/browser.html). For the
Python package, `maturin develop` in [`bindings/python`](bindings/python).

## The ten-minute path

```sh
python3 scripts/ten-minute-path.py
```

follows the quick start of [site/docs/index.html](site/docs/index.html) and the credit-decision
example page as a reader would: it runs each command the pages show and compares what it prints
with the output the pages show (and with `examples/credit-decision/expected-output.txt`). CI runs
it on a clean runner (job `ten-minute-path`). If you change a command's output, update the page:
`python3 site/tools/sync-examples.py` regenerates the example pages from the recorded outputs.

## The supply chain

- **Dependencies.** `cargo deny check` (configuration in [`deny.toml`](deny.toml)) fails on an
  open advisory, a licence outside the allowed list, a yanked crate, or a crate from anywhere but
  crates.io. CI runs it on every push (job `supply-chain`) together with `npm audit` of what the
  npm packages install. An exception goes in `deny.toml` with the advisory, why it does not apply
  here and what removes it.
- **SBOMs.** `scripts/sbom.sh` writes CycloneDX SBOMs for every crate (`cargo cyclonedx`) and both
  npm packages (`npm sbom`) into `dist/sbom/`.

## Releases and the supply chain

Publishing a GitHub release runs two workflows:

- [`release.yml`](.github/workflows/release.yml) runs `cargo deny` on the tagged commit, builds
  the npm tarballs in the pinned container, writes the SBOMs, attests every tarball and SBOM with
  a signed GitHub build-provenance attestation, and attaches them to the release.
- [`python-release.yml`](.github/workflows/python-release.yml) builds and attests every wheel and
  the sdist, attaches them to the release, and publishes to PyPI through Trusted Publishing (PyPI
  also receives PEP 740 attestations); see [bindings/python/RELEASING.md](bindings/python/RELEASING.md).

A consumer verifies a downloaded file against the workflow that built it with the GitHub CLI:

```sh
gh attestation verify vertep-web-0.1.11.tgz --repo Volland/vertep
gh attestation verify vertep-0.1.11-cp39-abi3-manylinux_2_17_x86_64.whl --repo Volland/vertep
```

**npm.** Publishing to npm is done by the maintainer, by hand. To publish with npm provenance
(the "Built and signed on GitHub Actions" badge, checked by `npm audit signatures`), publish the
attested tarballs from a workflow rather than from a laptop: `npm publish --provenance` only works
inside a supported CI run with `id-token: write`. When that is wanted:

1. On npmjs.com, add a trusted publisher for `@vertep/web` and `@vertep/dm-ecma` naming the
   repository `Volland/vertep` and the workflow `release.yml` (or add an `NPM_TOKEN` secret).
2. Add a job to `release.yml` that needs `npm`, downloads the `npm` artifact and runs
   `npm publish <tarball> --provenance --access public --tag next` for each tarball, with
   `permissions: id-token: write` and an environment with a required reviewer.

Until then the attestations on the release are the provenance of the npm tarballs.

**crates.io.** crates.io has no signatures and no provenance: it records a checksum for each
`.crate` file and nothing about where it was built. The release carries an SBOM for every crate,
attested like the other files; the crate files themselves are uploaded by the maintainer with
`cargo publish` and cannot be verified against a workflow.

## Checks before a pull request

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
tg check && tg ontology check        # the typed graph in tg.md/
openspec validate --all --strict      # the specs
```
