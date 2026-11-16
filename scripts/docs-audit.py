#!/usr/bin/env python3
"""Check that what the site and the roadmap say is built, or not yet built, agrees with the code.

A statement of availability carries a claim, which says what it asserts and where the code is:

    <span data-claim="built" data-ref="crates/vertep-agent/src/journal.rs#StoreAgentJournal">...</span>
    <li data-claim="missing" data-ref="openspec:production-readiness#Benchmark Suite">...</li>
    ... a sentence in markdown <!-- claim: missing openspec:production-readiness#Benchmark Suite -->

Any HTML element can carry `data-claim` (`built` or `missing`) and `data-ref`; the comment form
covers the line it is on, in markdown and HTML alike. `data-ref` holds one or more references,
separated by `;`:

    path                       the file or directory exists
    path#Symbol                Symbol is declared in the file, or in a file under the directory
                               (fn, struct, enum, trait, type, mod, const, class, def, function,
                               interface, a TypeScript method, a wasm-bindgen js_name, Lean
                               def/theorem/structure, or a file or directory named Symbol there)
    path#Type#method           both are declared there
    openspec:cap#Requirement   some code carries `@tg: implements::` for it without
                               `{confidence: partial}`; the requirement must exist in openspec/

A `built` claim holds when every reference resolves; a `missing` claim holds when none does (an
openspec requirement implemented only partially is still missing, and the file or directory of a
`path#Symbol` reference must exist, so a typo cannot pass silently). Claims on different pages
that share a reference must also say the same thing.

Then a sweep lists sentences outside any claim that use availability phrases ("not built", "not
yet", "planned", ...), so a new statement does not escape the audit unmarked. Those are warnings;
a reviewed sentence that is not about the code (a release on npm, prose) goes in
scripts/docs-audit-allow.txt.

    python3 scripts/docs-audit.py              # every claim, the code it found, and the warnings
    python3 scripts/docs-audit.py --check      # exit 1 on a contradicted or malformed claim (CI)
    python3 scripts/docs-audit.py --self-test  # prove both scenarios on a scratch copy (CI)

It needs only the standard library and reads the tree directly, so CI runs it without tg.
"""

import os
import re
import sys
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOW = "scripts/docs-audit-allow.txt"
KINDS = ("built", "missing")
CODE_EXT = {".rs", ".py", ".pyi", ".ts", ".tsx", ".js", ".mjs", ".lean", ".sh", ".toml", ".yml", ".yaml"}
SKIP_DIRS = {".git", "target", "node_modules", ".lake", "dist", "build", ".venv", "venv",
             "__pycache__", "pkg", "openspec", "tg.md", "Types", "Templates"}
PHRASES = re.compile(
    r"\bnot (?:yet )?(?:built|implemented|shipped|bound|wired|supported|available|there)\b"
    r"|\bnot yet\b|\bplanned\b|\bnot in the tree\b|\bcoming\b|\bnot there yet\b"
    r"|\bunimplemented\b", re.I)
COMMENT = re.compile(r"<!--\s*claim:\s*(\S+)\s+(.*?)\s*-->", re.S)
VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source",
        "track", "wbr"}
SKIP_TEXT = {"script", "style", "pre", "code"}


class Claim:
    def __init__(self, path, line, kind, refs, text=""):
        self.path, self.line, self.kind, self.refs, self.text = path, line, kind, refs, text

    @property
    def where(self):
        return f"{self.path}:{self.line}"


# --- reading the claims ---------------------------------------------------------------------

def split_refs(value):
    return [r.strip() for r in value.split(";") if r.strip()]


class PageParser(HTMLParser):
    """Collects the claims of one page and the text that no claim covers."""

    def __init__(self, path):
        super().__init__(convert_charrefs=True)
        self.path, self.claims, self.loose = path, [], []
        self.stack = []  # (tag, claim or None)
        self.comment_lines = set()

    def covered(self):
        return any(c is not None for _, c in self.stack)

    def skipped(self):
        return any(t in SKIP_TEXT for t, _ in self.stack)

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        claim = None
        if "data-claim" in a:
            claim = Claim(self.path, self.getpos()[0], a["data-claim"], split_refs(a.get("data-ref") or ""))
            self.claims.append(claim)
        if tag not in VOID:
            self.stack.append((tag, claim))

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag not in VOID and self.stack:
            self.stack.pop()

    def handle_endtag(self, tag):
        for i in range(len(self.stack) - 1, -1, -1):
            if self.stack[i][0] == tag:
                del self.stack[i:]
                return

    def handle_comment(self, data):
        m = COMMENT.match(f"<!--{data}-->")
        if m:
            line = self.getpos()[0]
            self.claims.append(Claim(self.path, line, m.group(1), split_refs(m.group(2))))
            self.comment_lines.add(line)

    def handle_data(self, data):
        for t in self.stack:
            if t[1] is not None:
                t[1].text += data
        if not self.covered() and not self.skipped() and data.strip():
            self.loose.append((self.getpos()[0], data))


def doc_files(root):
    files = sorted(p for p in (root / "site").rglob("*.html") if "tools" not in p.parts)
    roadmap = root / "tg.md" / "roadmap.md"
    return files + ([roadmap] if roadmap.exists() else [])


def strip_tags(line):
    return re.sub(r"\s+", " ", re.sub(r"<[^>]+>", "", line)).strip()


def sentence_at(text, start, end):
    left = max(text.rfind(". ", 0, start), text.rfind(": ", 0, start))
    right = text.find(". ", end)
    return text[left + 2 if left >= 0 else 0: right + 1 if right >= 0 else len(text)].strip()


def read_docs(root):
    """All claims, and the unmarked sentences that use an availability phrase."""
    claims, loose = [], []
    for path in doc_files(root):
        rel = str(path.relative_to(root))
        text = path.read_text()
        lines = text.splitlines()
        if path.suffix == ".html":
            parser = PageParser(rel)
            parser.feed(text)
            parser.close()
            claims += parser.claims
            for line, data in parser.loose:
                for m in PHRASES.finditer(data):
                    at = line + data.count("\n", 0, m.start())
                    if at in parser.comment_lines:
                        continue
                    whole = strip_tags(lines[at - 1]) if at <= len(lines) else data
                    phrase = re.sub(r"\s+", " ", m.group(0))
                    i = whole.find(phrase)
                    snippet = sentence_at(whole, i, i + len(phrase)) if i >= 0 else data.strip()
                    loose.append((rel, at, snippet))
        else:
            fence, front = False, lines[:1] == ["---"]
            for n, raw in enumerate(lines, 1):
                if front:
                    front = not (n > 1 and raw == "---")
                    continue
                if raw.startswith("```"):
                    fence = not fence
                if fence:
                    continue
                found = list(COMMENT.finditer(raw))
                for m in found:
                    claims.append(Claim(rel, n, m.group(1), split_refs(m.group(2)), COMMENT.sub("", raw)))
                if found:
                    continue
                plain = strip_tags(raw)
                for m in PHRASES.finditer(plain):
                    loose.append((rel, n, sentence_at(plain, m.start(), m.end())))
    return claims, loose


# --- resolving references against the code ---------------------------------------------------

def code_files(root):
    for dirpath, dirnames, filenames in os.walk(root):
        rel = Path(dirpath).relative_to(root)
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS and (d == ".github" or not d.startswith("."))
                       and not (rel == Path("site") and d != "tools")]
        for f in filenames:
            if Path(f).suffix in CODE_EXT:
                yield Path(dirpath) / f


ANNOTATION = re.compile(r"@tg:\s*implements::\s*\[\[openspec:([^\]#|]+)#([^\]#|]+)\]\]\s*(\{[^}\n]*\})?")


class Code:
    """The tree's implements annotations and spec requirements, read once."""

    def __init__(self, root):
        self.root = root
        self.implements = {}  # (cap, req) -> [(path, line, partial)]
        for path in code_files(root):
            try:
                text = path.read_text()
            except (UnicodeDecodeError, OSError):
                continue
            if "@tg:" not in text:
                continue
            for m in ANNOTATION.finditer(text):
                line = text.count("\n", 0, m.start()) + 1
                partial = bool(m.group(3) and re.search(r"confidence:\s*partial", m.group(3)))
                key = (m.group(1).strip(), m.group(2).strip())
                self.implements.setdefault(key, []).append((str(path.relative_to(root)), line, partial))
        self.requirements = set()
        for spec in (root / "openspec").rglob("spec.md") if (root / "openspec").exists() else []:
            cap = spec.parent.name
            for m in re.finditer(r"(?m)^### Requirement:\s*(.+?)\s*$", spec.read_text()):
                self.requirements.add((cap, m.group(1)))

    def requirement(self, ref):
        """(found, evidence, problem) for an openspec reference."""
        cap, _, req = ref[len("openspec:"):].partition("#")
        if (cap, req) not in self.requirements:
            return None, [], f"no requirement {req!r} in capability {cap!r} under openspec/"
        sites = self.implements.get((cap, req), [])
        full = [f"{p}:{n}" for p, n, partial in sites if not partial]
        part = [f"{p}:{n} (partial)" for p, n, partial in sites if partial]
        return bool(full), full or part, None

    def symbol(self, ref):
        """(found, evidence, problem) for a path or path#Symbol reference."""
        path, *names = ref.split("#")
        target = self.root / path
        if not target.exists():
            return False, [], f"{path} does not exist"
        if not names:
            return True, [path], None
        files = [target] if target.is_file() else sorted(
            p for p in target.rglob("*") if p.is_file() and p.suffix in CODE_EXT)
        evidence = []
        for name in names:
            hit = None
            if target.is_dir():
                named = [p for p in target.rglob("*") if p.stem == name or p.name == name]
                if named:
                    hit = str(named[0].relative_to(self.root))
            for f in files if hit is None else []:
                text = f.read_text(errors="replace")
                m = declaration(name, f.suffix).search(text)
                if m:
                    hit = f"{f.relative_to(self.root)}:{text.count(chr(10), 0, m.start()) + 1}"
                    break
            if hit is None:
                return False, evidence, None
            evidence.append(hit)
        return True, evidence, None

    def resolve(self, ref):
        return self.requirement(ref) if ref.startswith("openspec:") else self.symbol(ref)


def declaration(name, suffix):
    n = re.escape(name)
    forms = [rf"\b(?:fn|struct|enum|trait|type|mod|const|static|union|def|class|function|interface"
             rf"|theorem|lemma|structure|inductive|abbrev)\s+{n}\b",
             rf"\bmacro_rules!\s*{n}\b", rf"\bjs_name\s*=\s*\"{n}\""]
    if suffix in {".ts", ".tsx", ".js", ".mjs"}:
        forms.append(rf"^\s*(?:(?:export|public|private|protected|static|async|readonly|get|set)\s+)*"
                     rf"{n}\s*(?:<[^>\n]*>)?\s*\(")
        forms.append(rf"\b(?:const|let|var)\s+{n}\b")
    return re.compile("|".join(forms), re.M)


# --- the audit -------------------------------------------------------------------------------

def load_allow(root):
    path = root / ALLOW
    entries = []
    if path.exists():
        for n, raw in enumerate(path.read_text().splitlines(), 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            file, sep, fragment = line.partition(" | ")
            if sep:
                entries.append([file.strip(), norm(fragment), n, False])
    return entries


def norm(s):
    return re.sub(r"\s+", " ", s).strip().lower()


def audit(root):
    """Returns (claims with their results, problems, warnings)."""
    claims, loose = read_docs(root)
    code = Code(root)
    problems, results = [], []
    for c in claims:
        if c.kind not in KINDS:
            problems.append(f"{c.where}: claim kind {c.kind!r} is not one of {', '.join(KINDS)}")
            continue
        if not c.refs:
            problems.append(f"{c.where}: {c.kind} claim without a data-ref")
            continue
        for ref in c.refs:
            found, evidence, problem = code.resolve(ref)
            if problem:  # a typo or a stale reference: neither kind of claim can hold
                problems.append(f"{c.where}: {c.kind} claim refers to {ref}, but {problem}")
                continue
            if c.kind == "built" and not found:
                why = ("only partial implementations: " + ", ".join(evidence)) if evidence else "nothing in the code provides it"
                problems.append(f"{c.where}: says built, but {ref}: {why}")
            elif c.kind == "missing" and found:
                problems.append(f"{c.where}: says missing, but the code provides {ref} at {', '.join(evidence)}")
            results.append((c, ref, found, evidence))
    by_ref = {}
    for c in claims:
        for ref in c.refs:
            by_ref.setdefault(ref, []).append(c)
    for ref, group in sorted(by_ref.items()):
        kinds = {c.kind for c in group if c.kind in KINDS}
        if len(kinds) > 1:
            says = "; ".join(f"{c.where} says {c.kind}" for c in group)
            problems.append(f"pages disagree on {ref}: {says}")

    allow = load_allow(root)
    warnings = []
    for rel, line, snippet in loose:
        hit = [e for e in allow if e[0] == rel and e[1] in norm(snippet)]
        for e in hit:
            e[3] = True
        if not hit:
            warnings.append(f"{rel}:{line}: unmarked availability statement: \"{snippet}\"")
    for file, fragment, n, used in allow:
        if not used:
            warnings.append(f"{ALLOW}:{n}: allowlist entry matches nothing: {file} | {fragment}")
    return claims, results, problems, warnings


# @tg: implements:: [[openspec:production-readiness#Docs Match Behaviour]]
# @tg: implements:: [[decisions#Availability statements carry claims checked against the code]]
def main(argv, root=None):
    root = root or ROOT
    check = "--check" in argv
    claims, results, problems, warnings = audit(root)
    if not check:
        for c, ref, found, evidence in results:
            state = "found" if found else "absent"
            print(f"{c.where}: {c.kind} {ref}: {state}" + (f" ({', '.join(evidence)})" if evidence else ""))
    for w in warnings:
        print(f"warning: {w}", file=sys.stderr)
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    built = sum(c.kind == "built" for c in claims)
    print(f"{len(claims)} claims ({built} built, {len(claims) - built} missing) in "
          f"{len({c.path for c in claims})} files: {len(problems)} contradicted or malformed, "
          f"{len(warnings)} unmarked statements to review")
    if problems and check:
        sys.exit(1)
    return problems, warnings


def self_test():
    """Run both scenarios of the requirement on a scratch copy of the docs and the code."""
    import io
    import shutil
    import tempfile
    from contextlib import redirect_stderr, redirect_stdout

    def quiet(argv, root):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            try:
                main(argv, root)
            except SystemExit as e:
                return e.code, err.getvalue()
        return 0, err.getvalue()

    ignore = shutil.ignore_patterns("target", "node_modules", ".lake", ".git", "dist", "__pycache__", ".venv")
    with tempfile.TemporaryDirectory() as tmp:
        scratch = Path(tmp)
        for d in ("site", "crates", "bindings", "lean", "openspec", "scripts"):
            shutil.copytree(ROOT / d, scratch / d, ignore=ignore)
        (scratch / "tg.md").mkdir()
        shutil.copy(ROOT / "tg.md/roadmap.md", scratch / "tg.md/roadmap.md")
        assert quiet(["--check"], scratch)[0] == 0, "the scratch copy must pass before it is changed"

        # @tg: verifies:: [[openspec:production-readiness#Docs Match Behaviour#The agent journal statement is reconciled]] {level: integration}
        claims, results, problems, _ = audit(scratch)
        journal = "crates/vertep-agent/src/journal.rs#StoreAgentJournal"
        page = [c for c in claims if c.path == "site/examples/decider-agent.html" and journal in c.refs]
        roadmap = [c for c in claims if c.path == "tg.md/roadmap.md" and journal in c.refs]
        assert page and roadmap, "the decider-agent page and the roadmap both claim the agent journal"
        assert {c.kind for c in page + roadmap} == {"built"}, "they say the same thing"
        assert all(found for c, ref, found, _ in results if c in page + roadmap), "and the code has it"
        # The warning the page carried before the durable agent journal was delivered:
        before = ('<span data-claim="missing" data-ref="' + journal + '">The agent host keeps its own '
                  "journal, and only the in-memory <code>MemoryJournal</code> implements it today. A durable "
                  "agent journal on tiramemsu is not there yet.</span>")
        path = scratch / "site/examples/decider-agent.html"
        text = path.read_text()
        start = text.index('<span data-claim="built" data-ref="' + journal)
        stale = text[:start] + before + text[text.index("</span>", start) + len("</span>"):]
        path.write_text(stale)
        line = stale[:stale.index(before)].count("\n") + 1
        code, err = quiet(["--check"], scratch)
        assert code == 1, "a stale agent-journal warning passed the audit"
        assert f"site/examples/decider-agent.html:{line}: says missing" in err, err
        assert "crates/vertep-agent/src/journal.rs:" in err, err
        assert "pages disagree on " + journal in err and "tg.md/roadmap.md:" in err, err
        path.write_text(text)

        # @tg: verifies:: [[openspec:production-readiness#Docs Match Behaviour#An audit finds a stale claim]] {level: integration}
        bench = "openspec:production-readiness#Benchmark Suite"
        stale_pages = {c.where for c in claims if c.kind == "missing" and bench in c.refs}
        assert stale_pages, "some page says the benchmark suite is missing"
        lib = scratch / "crates/vertep-bench/src/lib.rs"
        lib.parent.mkdir(parents=True)
        tag = "@" + "tg"  # spelled apart, so this file does not itself implement the requirement
        lib.write_text(f"//! Benchmarks.\n\n// {tag}: implements:: [[{bench}]]\npub fn run() {{}}\n")
        code, err = quiet(["--check"], scratch)
        assert code == 1, "a page that says a delivered feature is missing passed the audit"
        for where in stale_pages:
            assert f"{where}: says missing, but the code provides {bench} at crates/vertep-bench/src/lib.rs:3" in err, err
        lib.unlink()
        # A new statement without a claim is surfaced as a warning, not a failure, and the
        # allowlist silences it once reviewed.
        why = scratch / "site/why.html"
        why.write_text(why.read_text().replace("</body>", "<p>Sharding by tenant is not built yet.</p></body>", 1))
        code, err = quiet(["--check"], scratch)
        assert code == 0 and 'unmarked availability statement: "Sharding by tenant is not built yet."' in err, err
        with (scratch / ALLOW).open("a") as f:
            f.write("site/why.html | Sharding by tenant is not built yet\n")
        assert "Sharding by tenant" not in quiet(["--check"], scratch)[1]
    print("self-test: a stale agent-journal warning and a page that says a delivered feature is missing "
          "fail the audit with the page line and the code location; an unmarked statement is a warning")


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        self_test()
    else:
        main(sys.argv[1:])
