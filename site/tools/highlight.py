#!/usr/bin/env python3
"""Syntax-highlight the Rust, XML (SCXML) and YAML code blocks of the site at build time.

The site ships no scripts, so highlighting is plain `<span class="k">` markup written into the
committed pages; the colours are in `assets/css/vertep.css` (section "code highlighting").
Needs Pygments (`pip install pygments`) only when this tool runs, never to view the site.

    python3 site/tools/highlight.py          # highlight every page in place; safe to repeat

`sync-examples.py` calls `highlight_block` too, so regenerated example pages stay highlighted.
"""

import re
import sys
from html import escape, unescape
from pathlib import Path

try:
    from pygments import lex
    from pygments.token import STANDARD_TYPES
    from pygments.lexers import RustLexer, XmlLexer, YamlLexer
except ImportError:  # pragma: no cover
    sys.exit("highlight.py needs Pygments: pip install pygments")

SITE = Path(__file__).resolve().parents[1]
LEXERS = {"rust": RustLexer, "scxml": XmlLexer, "xml": XmlLexer, "yaml": YamlLexer}
BLOCK = re.compile(r'<pre data-lang="([^"]*)"><code>(.*?)</code></pre>', re.S)


def lexer_for(lang: str):
    key = re.split(r"[\s(]", lang.strip(), maxsplit=1)[0].lower()
    cls = LEXERS.get(key)
    return cls(stripnl=False, ensurenl=False) if cls else None


PLAIN = {"", "w", "p", "n", "err"}  # left unmarked: whitespace, punctuation, plain names


def _css(ttype) -> str:
    while ttype not in STANDARD_TYPES:  # a rare subtype falls back to its parent's class
        ttype = ttype.parent
    css = STANDARD_TYPES[ttype]
    return "" if css in PLAIN else css


def highlight_block(text: str, lang: str) -> str:
    """Return the inner HTML of a code block: highlighted for Rust, XML and YAML, else escaped."""
    lexer = lexer_for(lang)
    if lexer is None:
        return escape(text)
    runs: list[list[str]] = []  # [css, text], adjacent tokens of one class merged
    for ttype, value in lex(text, lexer):
        css = _css(ttype)
        if runs and runs[-1][0] == css:
            runs[-1][1] += value
        else:
            runs.append([css, value])
    return "".join(
        f'<span class="{css}">{escape(v, quote=False)}</span>' if css else escape(v, quote=False)
        for css, v in runs
    )


def rewrite(html: str) -> str:
    def swap(m: re.Match) -> str:
        lang, body = m.group(1), m.group(2)
        if "<span" in body or lexer_for(lang) is None:
            return m.group(0)
        return f'<pre data-lang="{lang}"><code>{highlight_block(unescape(body), lang)}</code></pre>'

    return BLOCK.sub(swap, html)


def main() -> None:
    changed = 0
    for path in sorted(SITE.rglob("*.html")):
        old = path.read_text(encoding="utf-8")
        new = rewrite(old)
        if new != old:
            path.write_text(new, encoding="utf-8")
            changed += 1
    print(f"highlighted {changed} pages")


if __name__ == "__main__":
    main()
