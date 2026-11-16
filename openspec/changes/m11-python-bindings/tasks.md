## 1. Graph first

- [x] 1.1 Add milestone `M11 Python bindings` to `tg.md/roadmap.md` and its diagram
- [x] 1.2 Add Component `vertep-py` to `tg.md/components.md` with `provides::`, `uses::`, `realizes::` and `scheduled_in::` edges
- [x] 1.3 Add Capability `Python bindings` to `tg.md/capabilities.md` and list `python-bindings` in its `openspec:` frontmatter
- [x] 1.4 Record D1-D5 as `Decision` sections and the `PyPI releases are immutable` `Constraint`

## 2. Native module

- [x] 2.1 Create `crates/vertep-py` (PyO3, abi3-py39, `test = false`, own lints) with `//! @tg: implements::` for D1
- [x] 2.2 Implement `Core`: `command`, `pump`, `wait`, `now`, `next_deadline`, `export`, `close`, on the system or virtual clock
- [x] 2.3 Bind Python callables as guards and actions by IRI
- [x] 2.4 Open `storage="file"` with tiramemsu's native host

## 3. Python package

- [x] 3.1 `ActorSystem`, `Actor`, `Snapshot`, `Subscription` over the bridge, with dispatch that tolerates callbacks sending events
- [x] 3.2 One exception class per bridge error code, and `SystemClosed`
- [x] 3.3 Type stubs for the native module and a `py.typed` marker
- [x] 3.4 Tests proving each scenario of `python-bindings`, annotated with `@tg: verifies::`

## 4. Examples and docs

- [x] 4.1 Examples `quickstart`, `cart`, `durable_actor` and `guards_actions` with recorded outputs, run by the tests
- [x] 4.2 `site/docs/python.html`, linked from every docs sidebar, and the README
- [x] 4.3 List the examples in `examples/README.md`

## 5. Publishing

- [x] 5.1 `pyproject.toml` for maturin and the version gate `scripts/check_version.py`
- [x] 5.2 A `python` job in CI on the oldest and newest supported CPython
- [x] 5.3 The release workflow: wheels for five platforms, sdist, smoke test, TestPyPI dry run, PyPI Trusted Publishing
- [x] 5.4 `bindings/python/RELEASING.md`
- [x] 5.5 Reserve `vertep` on PyPI and TestPyPI with pending publishers and create the `pypi` and `testpypi` environments (the maintainer's, see RELEASING.md)
- [x] 5.6 Dry-run the release workflow to TestPyPI and confirm the wheels of every platform (superseded: the release workflow publishes to PyPI on a GitHub release; 0.1.9, 0.1.10 and 0.1.11 shipped 6 files each)
- [x] 5.7 Publish `0.1.6`, or the next release, to PyPI

## 6. Close

- [x] 6.1 Run `tg check`, `tg ontology check` and `tg trace python-bindings --gaps` and close every gap they report
