"""Fails when the Python package's version differs from the Rust workspace's.

The Rust workspace, the npm packages and the PyPI package share one version. Run from anywhere:
`python bindings/python/scripts/check_version.py`; pass a tag (`v0.1.6`) to check it as well.
"""

import re
import sys
from pathlib import Path

# @tg: implements:: [[openspec:python-bindings#Package Publishing]] {confidence: partial, note: "the version gate; the workflow is .github/workflows/python-release.yml"}
# @tg: implements:: [[openspec:python-bindings#Native Extension Module]] {confidence: partial, note: "the version gate"}
root = Path(__file__).resolve().parents[3]


def first_version(text, section):
    """The first `version = "..."` after the `[section]` header (no tomllib: it needs Python 3.11)."""
    body = text.split(f"[{section}]", 1)[1]
    return re.search(r'^version\s*=\s*"([^"]+)"', body, re.M).group(1)


workspace = first_version((root / "Cargo.toml").read_text(), "workspace.package")
package = first_version((root / "bindings/python/pyproject.toml").read_text(), "project")

problems = []
if package != workspace:
    problems.append(f"bindings/python/pyproject.toml is {package}, the workspace is {workspace}")
if len(sys.argv) > 1:
    tag = sys.argv[1].removeprefix("refs/tags/")
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:[-.+].+)?", tag) or tag[1:] != package:
        problems.append(f"the tag {tag} does not match the package version {package}")
if problems:
    print("\n".join(problems), file=sys.stderr)
    sys.exit(1)
print(f"vertep {package}: Cargo workspace and PyPI package agree")
