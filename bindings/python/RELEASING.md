# Releasing `vertep` to PyPI

The package is built by [maturin](https://www.maturin.rs/) from this directory, and published by
the workflow `.github/workflows/python-release.yml` with PyPI **Trusted Publishing**: GitHub proves
to PyPI who it is with a short-lived token, so no API token or password is stored in the repository.

The name `vertep` was free on both PyPI and TestPyPI on 2026-11-14. Names are first come, first
served, so do step 1 soon.

## One-time setup

### 1. Accounts

1. Create an account on <https://pypi.org/account/register/> and another on
   <https://test.pypi.org/account/register/> (they are separate sites with separate accounts).
2. Turn on two-factor authentication on both. PyPI requires it to publish.
3. Verify the email address.

### 2. Reserve the name with a pending publisher

A trusted publisher can be registered *before the project exists*; the first upload then creates
it, owned by your account. On each site, open **Your account → Publishing → Add a new pending
publisher** and fill in:

| Field | PyPI | TestPyPI |
|---|---|---|
| PyPI project name | `vertep` | `vertep` |
| Owner | `Volland` | `Volland` |
| Repository name | `vertep` | `vertep` |
| Workflow name | `python-release.yml` | `python-release.yml` |
| Environment name | `pypi` | `testpypi` |

The values must match the workflow exactly; a mismatch shows up as "invalid-publisher" at upload
time. (PyPI: <https://pypi.org/manage/account/publishing/>; TestPyPI:
<https://test.pypi.org/manage/account/publishing/>.)

### 3. GitHub environments

In the GitHub repository open **Settings → Environments** and create two environments named
`pypi` and `testpypi`. On `pypi`, add **Required reviewers** (yourself) so that no upload happens
without a click, even from a published release.

Nothing else needs configuring: there are no secrets to add.

## Every release

The Rust workspace, the npm packages and the PyPI package share one version. To release `0.1.6`:

1. Change the version in the workspace `Cargo.toml` (`[workspace.package]` and the
   `[workspace.dependencies]` entries), in `bindings/python/pyproject.toml`, and in the npm
   packages as usual. `python bindings/python/scripts/check_version.py` fails if the PyPI package
   and the workspace differ.
2. Add the release to `CHANGELOG.md` and merge to `main`. CI (`Python bindings on Python 3.9 and
   3.13`) must be green.
3. **Dry run** the first time and after any change to the build: *Actions → Python release → Run
   workflow* with "Publish to TestPyPI" ticked. It builds the wheels for Linux (x86_64, aarch64),
   macOS (x86_64, arm64) and Windows (x64), smoke-tests each in a clean virtual environment, and
   uploads everything to TestPyPI after the `testpypi` environment allows it. Then, in a fresh
   virtual environment:

   ```sh
   pip install --index-url https://test.pypi.org/simple/ --no-deps vertep==0.1.6
   python -c "import vertep; print(vertep.__version__)"
   ```

   Versions on PyPI and TestPyPI can never be re-uploaded or replaced, so bump the version rather
   than retrying a published one.
4. Tag and publish a GitHub release named `v0.1.6` (*Releases → Draft a new release*). The
   workflow checks that the tag equals the package version, builds, smoke-tests, waits for the
   `pypi` environment's approval and uploads the wheels and the sdist to PyPI.
5. Check <https://pypi.org/project/vertep/> and `pip install vertep` in a clean environment.

A published release can be **yanked** on PyPI (it stays installable by exact pin but is skipped by
`pip install vertep`); it cannot be deleted and re-uploaded.

## What is shipped

- **Wheels**, one per platform, built against the stable ABI (`abi3-py39`): the same file works
  on every CPython from 3.9 on, so there is no wheel per Python version.
- **An sdist**, which builds with a Rust toolchain (the pinned 1.95) for platforms without a
  wheel; the build takes about a minute.
- The default build holds the core, the CEL and `rust` datamodels and tiramemsu. The ECMAScript
  datamodel (boa) is the cargo feature `ecmascript` and is not in the published wheels; add
  `[tool.maturin] features = ["ecmascript"]` to build a variant.

## Manual build and upload (without CI)

For an emergency, or to try the whole path locally:

```sh
cd bindings/python
maturin build --release --out dist          # a wheel for this machine
maturin sdist --out dist
pip install twine
twine upload --repository testpypi dist/*   # asks for a TestPyPI API token (user: __token__)
```

API tokens are the fallback, not the plan: create them under *Account settings → API tokens*,
scope them to the `vertep` project once it exists, and never commit them.

## Troubleshooting

- **`invalid-publisher` at upload.** The workflow file name, the environment name, the owner or
  the repository in the pending publisher does not match the run. Compare the claims shown in the
  error with the table above.
- **The release job did not start.** The publish job runs only for the `published` release event;
  a draft does not trigger it.
- **A platform's wheel failed.** The matrix has `fail-fast: false`, so the others still built. Fix
  the platform, then re-run the failed jobs; publishing waits for all of them.
- **`File already exists` at upload.** That version is on the index. Bump the version.
