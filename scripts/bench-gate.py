#!/usr/bin/env python3
# @tg: implements:: [[openspec:runtime-performance#Benchmark Regression Gate]]
# @tg: implements:: [[openspec:production-readiness#Benchmark Suite]] {confidence: partial}
"""The benchmark regression gate: collect, record and compare `vertep-bench` results.

    bench-gate.py collect [--out FILE] [--sizes FILE]
        Reads criterion's estimates under target/criterion and writes one results file with
        the commit, the machine and the durability mode (benches/runtime/src/lib.rs).

    bench-gate.py record --results FILE --reason TEXT [--baseline benches/baseline.json]
        Makes FILE the baseline. A baseline only moves with a recorded reason; the reason and
        the previous commit are kept in the baseline's history.

    bench-gate.py page [--check]
        Renders the recorded figures of the baseline into site/docs/benchmarks.html, between
        the `bench:begin` and `bench:end` markers; --check fails when the page is stale.

    bench-gate.py compare --base FILE --head FILE [--baseline benches/baseline.json]
        Fails (exit 1) when a case in HEAD is slower than in BASE by more than its threshold.
        Thresholds and accepted changes come from the baseline file: `thresholds` maps a case
        id prefix to a fraction (longest prefix wins, `default` otherwise), and `accepted`
        lists case ids whose move is on purpose, each with a reason.

CI runs the suite at the last release tag and at the change on the same runner, so the
comparison does not depend on the machine; benches/baseline.json carries the thresholds, the
accepted changes and the recorded figures that the results page shows.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRITERION = ROOT / "target" / "criterion"
BASELINE = ROOT / "benches" / "baseline.json"
LIB = ROOT / "benches" / "runtime" / "src" / "lib.rs"


def git(*args: str) -> str:
    try:
        return subprocess.run(
            ["git", *args], cwd=ROOT, capture_output=True, text=True, check=True
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def machine() -> dict:
    cpu = platform.processor() or platform.machine()
    try:
        if sys.platform == "darwin":
            cpu = subprocess.run(
                ["sysctl", "-n", "machdep.cpu.brand_string"],
                capture_output=True, text=True, check=True,
            ).stdout.strip() or cpu
        elif Path("/proc/cpuinfo").exists():
            for line in Path("/proc/cpuinfo").read_text().splitlines():
                if line.startswith("model name"):
                    cpu = line.split(":", 1)[1].strip()
                    break
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    return {
        "os": f"{platform.system()} {platform.release()}",
        "arch": platform.machine(),
        "cpu": cpu,
        "cores": os.cpu_count(),
        "runner": os.environ.get("RUNNER_NAME", "local"),
    }


def durability() -> str:
    m = re.search(r'TIRAMEMSU_DURABILITY: &str = "([^"]+)"', LIB.read_text())
    return m.group(1) if m else "unknown"


def read_cases(root: Path) -> dict:
    cases = {}
    for bench in sorted(root.rglob("new/benchmark.json")):
        meta = json.loads(bench.read_text())
        est_file = bench.parent / "estimates.json"
        if not est_file.exists():
            continue
        est = json.loads(est_file.read_text())
        case = meta.get("full_id") or meta.get("title")
        tp = meta.get("throughput") or {}
        elements = tp.get("Elements") or tp.get("elements")
        cases[case] = {
            "mean_ns": round(est["mean"]["point_estimate"], 1),
            "ci_low_ns": round(est["mean"]["confidence_interval"]["lower_bound"], 1),
            "ci_high_ns": round(est["mean"]["confidence_interval"]["upper_bound"], 1),
            **({"elements": elements} if elements else {}),
        }
    return cases


# @tg: verifies:: [[openspec:runtime-performance#Hot Path Benchmarks#One command runs the suite]] {level: e2e}
def collect(args) -> int:
    root = Path(args.criterion)
    cases = read_cases(root)
    if not cases:
        print(f"no criterion results under {root}; run `cargo bench -p vertep-bench` first",
              file=sys.stderr)
        return 2
    results = {
        "commit": git("rev-parse", "HEAD"),
        "describe": git("describe", "--tags", "--always", "--dirty"),
        "date": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "machine": machine(),
        "durability": {"memory": "volatile", "tiramemsu": durability()},
        "quick": bool(os.environ.get("VERTEP_BENCH_QUICK")),
        "cases": cases,
    }
    if args.sizes:
        results["sizes"] = json.loads(Path(args.sizes).read_text())
    text = json.dumps(results, indent=2, sort_keys=True) + "\n"
    if args.out:
        Path(args.out).write_text(text)
        print(f"{len(cases)} cases written to {args.out}")
    else:
        sys.stdout.write(text)
    return 0


def load(path: Path) -> dict:
    return json.loads(path.read_text()) if path.exists() else {}


def record(args) -> int:
    if not args.reason.strip():
        print("a baseline moves only with a reason", file=sys.stderr)
        return 2
    path = Path(args.baseline)
    old = load(path)
    new = json.loads(Path(args.results).read_text())
    history = old.get("history", [])
    if old.get("figures"):
        history.append({
            "commit": old["figures"].get("commit"),
            "date": old["figures"].get("date"),
            "reason": old.get("reason"),
        })
    baseline = {
        "thresholds": old.get("thresholds", {"default": 0.25}),
        "accepted": old.get("accepted", {}),
        "reason": args.reason,
        "figures": new,
        "history": history,
    }
    path.write_text(json.dumps(baseline, indent=2, sort_keys=True) + "\n")
    print(f"baseline recorded in {path}: {len(new.get('cases', {}))} cases ({args.reason})")
    return 0


def threshold(thresholds: dict, case: str) -> float:
    best, value = -1, thresholds.get("default", 0.25)
    for prefix, t in thresholds.items():
        if prefix != "default" and case.startswith(prefix) and len(prefix) > best:
            best, value = len(prefix), t
    return float(value)


def compare_results(base: dict, head: dict, baseline: dict) -> tuple[list, list]:
    thresholds = baseline.get("thresholds", {"default": 0.25})
    accepted = baseline.get("accepted", {})
    failures, lines = [], []
    for case, h in sorted(head.get("cases", {}).items()):
        b = base.get("cases", {}).get(case)
        if b is None:
            lines.append(f"  new      {case}: {h['mean_ns']:.0f} ns")
            continue
        change = h["mean_ns"] / b["mean_ns"] - 1 if b["mean_ns"] else 0.0
        limit = threshold(thresholds, case)
        status = "ok"
        if change > limit:
            if case in accepted:
                status = "accepted"
            else:
                status = "REGRESSED"
                failures.append((case, b["mean_ns"], h["mean_ns"], change, limit))
        lines.append(
            f"  {status:<9}{case}: base {b['mean_ns']:.0f} ns, now {h['mean_ns']:.0f} ns "
            f"({change:+.1%}, threshold {limit:.0%})"
        )
    return failures, lines


def compare(args) -> int:
    base = load(Path(args.base))
    head = load(Path(args.head))
    if not base.get("cases"):
        print(f"no results at the base ({args.base}): the gate has nothing to compare against")
        return 0
    failures, lines = compare_results(base, head, load(Path(args.baseline)))
    print(f"base {base.get('describe')} vs head {head.get('describe')}")
    print("\n".join(lines))
    if failures:
        print("\nbenchmark regression:", file=sys.stderr)
        for case, b, h, change, limit in failures:
            print(f"  {case}: baseline {b:.0f} ns, new {h:.0f} ns ({change:+.1%} > {limit:.0%})",
                  file=sys.stderr)
        print("move the baseline on purpose by listing the case under `accepted` in "
              "benches/baseline.json with a reason", file=sys.stderr)
        return 1
    return 0


# @tg: verifies:: [[openspec:runtime-performance#Benchmark Regression Gate#Regression blocks the merge]] {level: unit}
# @tg: verifies:: [[openspec:runtime-performance#Benchmark Regression Gate#Accepted baseline change]] {level: unit}
# @tg: verifies:: [[openspec:production-readiness#Benchmark Suite#A regression fails CI]] {level: unit}
PAGE = ROOT / "site" / "docs" / "benchmarks.html"
BEGIN, END = "<!-- bench:begin -->", "<!-- bench:end -->"


def fmt_ns(ns: float) -> str:
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if ns >= scale:
            return f"{ns / scale:.3g} {unit}"
    return f"{ns:.3g} ns"


def esc(text: str) -> str:
    return (str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
            .replace('"', "&quot;"))


def render(baseline: dict) -> str:
    f = baseline.get("figures", {})
    m = f.get("machine", {})
    d = f.get("durability", {})
    out = [BEGIN]
    out.append(
        f'    <p class="bench-meta">Commit <code>{esc(f.get("commit", "?")[:12])}</code> '
        f'({esc(f.get("describe", ""))}), recorded {esc(f.get("date", "?"))} on '
        f'{esc(m.get("cpu", "?"))}, {esc(m.get("cores", "?"))} cores, {esc(m.get("os", "?"))}. '
        f'Tiramemsu durability: <code>{esc(d.get("tiramemsu", "?"))}</code>. '
        f'Reason for this baseline: {esc(baseline.get("reason", "?"))}</p>'
    )
    groups: dict = {}
    for case, c in sorted(f.get("cases", {}).items()):
        groups.setdefault(case.split("/", 1)[0], []).append((case, c))
    for group, cases in groups.items():
        out.append(f'    <h3 id="bench-{esc(group)}">{esc(group)}</h3>')
        out.append('    <table class="bench">')
        out.append("      <thead><tr><th>Case</th><th>Mean</th><th>95% interval</th>"
                   "<th>Per second</th></tr></thead>")
        out.append("      <tbody>")
        for case, c in cases:
            per = c.get("elements", 1) * 1e9 / c["mean_ns"] if c["mean_ns"] else 0
            out.append(
                f"        <tr><td><code>{esc(case)}</code></td><td>{fmt_ns(c['mean_ns'])}</td>"
                f"<td>{fmt_ns(c['ci_low_ns'])} – {fmt_ns(c['ci_high_ns'])}</td>"
                f"<td>{per:,.0f}</td></tr>"
            )
        out.append("      </tbody>")
        out.append("    </table>")
    sizes = f.get("sizes")
    if sizes:
        out.append('    <h3 id="bench-sizes">Package size and cold start</h3>')
        out.append('    <table class="bench">')
        out.append("      <thead><tr><th>Figure</th><th>Value</th></tr></thead>")
        out.append("      <tbody>")
        for k, v in sorted(sizes.items()):
            out.append(f"        <tr><td><code>{esc(k)}</code></td>"
                       f"<td>{'not measured' if v is None else esc(v)}</td></tr>")
        out.append("      </tbody>")
        out.append("    </table>")
    out.append("    " + END)
    return "\n".join(out)


def page(args) -> int:
    baseline = load(Path(args.baseline))
    text = PAGE.read_text()
    start, end = text.find(BEGIN), text.find(END)
    if start < 0 or end < 0:
        print(f"{PAGE} has no {BEGIN} ... {END} markers", file=sys.stderr)
        return 2
    new = text[:start] + render(baseline) + text[end + len(END):]
    if args.check:
        if new != text:
            print(f"{PAGE} is stale: run scripts/bench-gate.py page", file=sys.stderr)
            return 1
        print("benchmarks page matches the baseline")
        return 0
    PAGE.write_text(new)
    print(f"{PAGE} written")
    return 0


# @tg: verifies:: [[openspec:production-readiness#Benchmark Suite#Results are recorded]] {level: unit}
def page_self_test() -> None:
    html = render({"reason": "first", "figures": {
        "commit": "abc", "machine": {"cpu": "c"}, "durability": {"tiramemsu": "normal"},
        "cases": {"turns/memory/no_send": {"mean_ns": 2000.0, "ci_low_ns": 1.0,
                                           "ci_high_ns": 3.0}}}})
    assert "Commit <code>abc</code>" in html and "c, " in html, html
    assert "turns/memory/no_send" in html and "500,000" in html, html


def self_test() -> int:
    base = {"cases": {"turns/memory/no_send": {"mean_ns": 100.0},
                      "journal/replay_1000/tiramemsu": {"mean_ns": 1000.0}}}
    head = {"cases": {"turns/memory/no_send": {"mean_ns": 140.0},
                      "journal/replay_1000/tiramemsu": {"mean_ns": 1100.0},
                      "new/case": {"mean_ns": 1.0}}}
    fails, _ = compare_results(base, head, {"thresholds": {"default": 0.25}})
    assert [f[0] for f in fails] == ["turns/memory/no_send"], fails
    fails, _ = compare_results(base, head, {"thresholds": {"default": 0.25, "turns/": 0.5}})
    assert not fails, fails
    fails, _ = compare_results(base, head, {"accepted": {"turns/memory/no_send": "on purpose"}})
    assert not fails, fails
    assert threshold({"default": 0.1, "a/": 0.2, "a/b": 0.3}, "a/b/c") == 0.3
    page_self_test()
    print("bench-gate self-test ok")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("collect")
    c.add_argument("--criterion", default=str(CRITERION))
    c.add_argument("--out")
    c.add_argument("--sizes")
    r = sub.add_parser("record")
    r.add_argument("--results", required=True)
    r.add_argument("--reason", required=True)
    r.add_argument("--baseline", default=str(BASELINE))
    k = sub.add_parser("compare")
    k.add_argument("--base", required=True)
    k.add_argument("--head", required=True)
    k.add_argument("--baseline", default=str(BASELINE))
    g = sub.add_parser("page")
    g.add_argument("--check", action="store_true")
    g.add_argument("--baseline", default=str(BASELINE))
    sub.add_parser("self-test")
    args = p.parse_args()
    return {"collect": collect, "record": record, "compare": compare, "page": page,
            "self-test": lambda _a: self_test()}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
