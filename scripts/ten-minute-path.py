#!/usr/bin/env python3
"""Follows the site's quick start and the credit-decision example as a reader would, and fails
when a step does not run or prints something other than what the page shows.

The commands and the outputs are read from the published pages themselves, not copied here, so a
change that breaks a documented step, or a page that drifts from the code, fails CI and names the
step (M13 production-readiness, "Ten Minute Path"). CI runs it on a clean runner
(.github/workflows/ci.yml, job `ten-minute-path`):

    python3 scripts/ten-minute-path.py           # every step, except the full test suite
    python3 scripts/ten-minute-path.py --full    # also `cargo test --workspace` (nightly)
    python3 scripts/ten-minute-path.py --self-test   # a drifted page must fail and name the step

What it does with each block of a page, in order:
  - a Shell block: runs each line from the repository root (`vt` resolves to the binary the page
    just built, and runs in a scratch directory that holds the files the page told you to save);
  - the Output block after it: compares with what those commands printed on standard output;
  - an SCXML block the page asks you to save ("Save it as <code>order.scxml</code>"): saves it.
`git clone` and `cd vertep` are satisfied by the checkout the job runs in. The credit-decision
output is also compared with examples/credit-decision/expected-output.txt, byte for byte, as
`cargo test -p vertep-examples` does.
"""

# @tg: implements:: [[openspec:production-readiness#Ten Minute Path]]

from __future__ import annotations

import difflib
import html
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# (page, first anchor, anchor that ends the path or None for the end of the page)
PAGES = [
    ("site/docs/index.html", "build", "own-project"),
    ("site/examples/credit-decision.html", "run-it", None),
]
RECORDED = {
    "cargo run -p vertep-examples --bin credit-decision": "examples/credit-decision/expected-output.txt",
}

BLOCK = re.compile(r'<pre(?P<cls> class="output")? data-lang="(?P<lang>[^"]+)"><code>(?P<body>.*?)</code></pre>', re.S)
ANCHOR = re.compile(r'<h[23] id="(?P<id>[^"]+)"')
SAVE_AS = re.compile(r"Save it as <code>(?P<name>[^<]+)</code>")


@dataclass
class Block:
    lang: str
    text: str
    anchor: str
    save_as: str | None


class StepFailed(Exception):
    pass


def text_of(body: str) -> str:
    return html.unescape(re.sub(r"<[^>]+>", "", body))


def blocks(page: str, first: str, last: str | None) -> list[Block]:
    source = (ROOT / page).read_text()
    start = source.index(f'id="{first}"')
    end = source.index(f'id="{last}"') if last else len(source)
    region = source[start:end]
    out, pos = [], 0
    for m in BLOCK.finditer(region):
        before = region[: m.start()]
        anchors = ANCHOR.findall(before)
        save = SAVE_AS.findall(region[pos : m.start()])
        out.append(Block(m["lang"], text_of(m["body"]), anchors[-1] if anchors else first, save[-1] if save else None))
        pos = m.end()
    return out


def normalise(text: str) -> str:
    return "\n".join(line.rstrip() for line in text.strip("\n").splitlines())


def run(cmd: str, where: str, scratch: Path, env: dict[str, str]) -> str:
    cwd = scratch if cmd.startswith("vt ") else ROOT
    print(f"  $ {cmd}", flush=True)
    r = subprocess.run(cmd, shell=True, cwd=cwd, env=env, stdout=subprocess.PIPE, text=True)
    if r.returncode != 0:
        raise StepFailed(f"step `{cmd}` ({where}) exited with {r.returncode}")
    return r.stdout


def follow(page: str, first: str, last: str | None, scratch: Path, full: bool) -> int:
    env = dict(os.environ)
    env["PATH"] = f"{ROOT / 'target' / 'release'}{os.pathsep}{env.get('PATH', '')}"
    steps = 0
    printed: str | None = None
    ran: list[str] = []
    for b in blocks(page, first, last):
        where = f"{page}#{b.anchor}"
        if b.lang == "Shell":
            printed, ran = "", []
            for line in b.text.splitlines():
                cmd = re.sub(r"\s+#.*$", "", line).strip()
                if not cmd:
                    continue
                if cmd.startswith("git clone ") or cmd == "cd vertep":
                    print(f"  - {cmd}  (the checkout this runs in)")
                    continue
                if cmd == "cargo test --workspace" and not full:
                    print(f"  - {cmd}  (the `rust` CI job runs it; --full runs it here)")
                    continue
                printed += run(cmd, where, scratch, env)
                ran.append(cmd)
                steps += 1
                if cmd in RECORDED:
                    check_recorded(cmd, where, printed)
        elif b.lang == "Output":
            if printed is None:
                raise StepFailed(f"{where}: an Output block with no command before it")
            if normalise(printed) != normalise(b.text):
                diff = "\n".join(
                    difflib.unified_diff(
                        normalise(b.text).splitlines(), normalise(printed).splitlines(), "the page", "the command", lineterm=""
                    )
                )
                raise StepFailed(f"step `{' && '.join(ran)}` ({where}) printed something other than the page shows:\n{diff}")
            print(f"  = output matches {where}")
            printed = None
        elif b.save_as:
            (scratch / b.save_as).write_text(b.text if b.text.endswith("\n") else b.text + "\n")
            print(f"  > saved {b.save_as} from {where}")
    return steps


def check_recorded(cmd: str, where: str, printed: str) -> None:
    rel = RECORDED[cmd]
    if printed != (ROOT / rel).read_text():
        diff = "\n".join(
            difflib.unified_diff((ROOT / rel).read_text().splitlines(), printed.splitlines(), rel, "the command", lineterm="")
        )
        raise StepFailed(f"step `{cmd}` ({where}) differs from {rel}:\n{diff}")
    print(f"  = output equals {rel}")


# @tg: verifies:: [[openspec:production-readiness#Ten Minute Path#A broken quick start fails CI]] {level: integration}
def self_test() -> int:
    """A page whose documented command prints something else, and one whose command fails, must
    each fail and name the step and the page anchor."""
    page = """<h2 id="start">Start</h2>
<pre data-lang="Shell"><code>echo vertep</code></pre>
<pre class="output" data-lang="Output"><code>%s</code></pre>"""
    with tempfile.TemporaryDirectory(prefix="vertep-ten-minute-self-test-") as tmp:
        cases = [("vertep", None), ("something else", "step `echo vertep` (%s#start) printed something other")]
        for i, (shown, expected) in enumerate(cases):
            path = Path(tmp) / f"page{i}.html"
            path.write_text(page % shown)
            try:
                follow(str(path), "start", None, Path(tmp), False)
                error = None
            except StepFailed as e:
                error = str(e)
            if expected is None and error is not None:
                print(f"self-test: a matching page failed: {error}", file=sys.stderr)
                return 1
            if expected is not None and (error is None or not error.startswith(expected % path)):
                print(f"self-test: a drifted page gave {error!r}", file=sys.stderr)
                return 1
        path = Path(tmp) / "failing.html"
        path.write_text(page.replace("echo vertep", "exit 3") % "")
        try:
            follow(str(path), "start", None, Path(tmp), False)
            print("self-test: a failing command passed", file=sys.stderr)
            return 1
        except StepFailed as e:
            if not str(e).startswith(f"step `exit 3` ({path}#start) exited with 3"):
                print(f"self-test: a failing command gave {e}", file=sys.stderr)
                return 1
    print("self-test: a drifted output and a failing command each fail and name the step")
    return 0


# @tg: verifies:: [[openspec:production-readiness#Ten Minute Path#The quick start runs]] {level: e2e}
def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    full = "--full" in argv
    total = 0
    with tempfile.TemporaryDirectory(prefix="vertep-ten-minute-") as tmp:
        for page, first, last in PAGES:
            print(f"{page} from #{first}", flush=True)
            try:
                total += follow(page, first, last, Path(tmp), full)
            except StepFailed as e:
                print(f"ten-minute path: FAILED: {e}", file=sys.stderr)
                return 1
    print(f"ten-minute path: {total} documented commands ran and printed what the pages show")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
