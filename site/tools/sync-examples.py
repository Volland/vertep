#!/usr/bin/env python3
"""Write site/examples/*.html from the runnable examples in examples/.

The site has no build step: the pages are committed as plain HTML. This script only copies the
example sources and their recorded outputs (the files `cargo test -p vertep-examples` checks)
into the pages, so the code and output on the site are the tested ones. Run it from anywhere
after the examples change:

    python3 site/tools/sync-examples.py
"""

from html import escape
from pathlib import Path

from highlight import highlight_block

ROOT = Path(__file__).resolve().parents[2]
EX = ROOT / "examples"
OUT = ROOT / "site" / "examples"

FONTS = "../assets/css/fonts.css"  # self-hosted, no third-party request


def read(rel: str) -> str:
    return (EX / rel).read_text().rstrip("\n")


def excerpt(rel: str, first: int, last: int) -> str:
    lines = read(rel).splitlines()
    return "\n".join(lines[first - 1 : last])


def code(text: str, lang: str, output: bool = False) -> str:
    cls = ' class="output"' if output else ""
    body = escape(text) if output else highlight_block(text, lang)
    return f'<pre{cls} data-lang="{escape(lang)}"><code>{body}</code></pre>'


def run(cmd: str) -> str:
    return code(cmd, "Shell")


# Each example: slug, title, numeral, one-line summary, lede, and the page body as a list of
# ("p", html) / ("h2", text) / ("code", (text, lang)) / ("out", (text, lang)) / ("ul", [html])
# / ("nb", html) / ("warn", html) blocks. Prose is hand-written; code and output are read.
EXAMPLES = [
    {
        "slug": "traffic-light",
        "title": "Traffic light",
        "num": "I",
        "summary": "An SCXML chart driven through the pure core: a parallel state, delayed sends, a host that owns the clock.",
        "lede": "A traffic light with a pedestrian crossing, run through the core interpreter by a host of about forty lines. It shows what it means for <code>step</code> to be pure: the chart asks for timers, and the host keeps them.",
        "files": ["traffic-light/traffic-light.scxml", "traffic-light/main.rs", "traffic-light/expected-output.txt", "traffic-light/vt/"],
        "body": lambda: [
            ("h2", "The chart"),
            ("p", "<code>operating</code> is a parallel state. Its <code>light</code> region cycles red, green and yellow on delayed <code>&lt;send&gt;</code>s named <code>light-timer</code>, and cancels the pending one on exit. Its <code>crossing</code> region follows the light with eventless <code>In('red')</code> transitions. A pedestrian <code>button</code> cuts green short, and <code>fault</code> leaves the whole parallel state for <code>flashing</code>."),
            ("code", (read("traffic-light/traffic-light.scxml"), "SCXML")),
            ("h2", "The host"),
            ("p", "<code>session::start</code> and <code>session::step</code> return the next session and a list of effects such as “deliver <code>tick</code> in 30 s” or “cancel <code>light-timer</code>”. They never sleep or read a clock. The program keeps the timers on a virtual clock and feeds them back as events, so the run is the same every time."),
            ("code", (read("traffic-light/main.rs"), "Rust")),
            ("h2", "Run it"),
            ("raw", run("cargo run -p vertep-examples --bin traffic-light")),
            ("out", (read("traffic-light/expected-output.txt"), "Output")),
            ("h2", "The same chart through vt"),
            ("p", "Every surface of a chart has the same content-addressed version. The YAML and Turtle that <code>vt convert</code> writes hash to the same IRI as the SCXML, and a test checks it."),
            ("raw", run("vt validate examples/traffic-light/traffic-light.scxml\nvt hash examples/traffic-light/traffic-light.scxml")),
            ("out", (read("traffic-light/vt/validate.txt") + "\n" + read("traffic-light/vt/hash.txt"), "Output")),
            ("raw", run("vt convert --to yaml examples/traffic-light/traffic-light.scxml")),
            ("out", (excerpt("traffic-light/vt/traffic-light.yaml", 1, 30), "YAML, first 30 lines")),
            ("raw", run("vt convert --to turtle examples/traffic-light/traffic-light.scxml")),
            ("out", (excerpt("traffic-light/vt/traffic-light.ttl", 1, 22), "Turtle, first 22 lines")),
            ("ul", [
                "Delays accept <code>s</code> and <code>ms</code>.",
                "Every state, transition and target is a node minted under the version IRI, with no blank nodes.",
            ]),
        ],
    },
    {
        "slug": "typed-order",
        "title": "Typed order",
        "num": "II",
        "summary": "A typed YAML chart on CEL: vt typecheck, vt schema, vt gen ts, and a type error with its location.",
        "lede": "An order chart in YAML on the CEL datamodel, with a <code>types</code> block: records, a union, <code>optional</code> and <code>list</code>. The <code>vt</code> command checks every guard and assignment against the types and projects them to JSON Schema, SHACL, CEL and TypeScript.",
        "files": ["typed-order/order.yaml", "typed-order/order-ill-typed.yaml", "typed-order/vt/"],
        "body": lambda: [
            ("h2", "The chart"),
            ("code", (read("typed-order/order.yaml"), "YAML")),
            ("h2", "Type-check it"),
            ("raw", run("vt typecheck examples/typed-order/order.yaml")),
            ("out", (read("typed-order/vt/typecheck.txt"), "Output")),
            ("p", "<code>order-ill-typed.yaml</code> assigns the item's <code>sku</code>, a string, to <code>ctx.total</code>, a decimal. The error names the file, line and column, the path in the document, the state and the transition:"),
            ("raw", run("vt typecheck examples/typed-order/order-ill-typed.yaml   # exit code 1")),
            ("out", (read("typed-order/vt/typecheck-ill-typed.txt"), "Output")),
            ("h2", "Project the types"),
            ("p", "One type language, several projections. The CEL type environment is what the checker uses for expressions:"),
            ("raw", run("vt schema --format cel examples/typed-order/order.yaml")),
            ("out", (read("typed-order/vt/schema-cel.txt"), "Output")),
            ("raw", run("vt schema --format jsonschema examples/typed-order/order.yaml")),
            ("out", (excerpt("typed-order/vt/schema.json", 1, 40), "JSON Schema, first 40 lines")),
            ("h2", "Generate TypeScript"),
            ("p", "<code>vt gen ts</code> writes a module for <code>@vertep/web</code>: the event union, the context interface, the state names and a typed <code>createActor</code>."),
            ("raw", run("vt gen ts examples/typed-order/order.yaml")),
            ("out", (ts_excerpt(), "TypeScript, excerpt")),
            ("nb", "Every host loads a chart that declares types with its typed CEL datamodel: the actor runtime, the agent host and the browser worker refuse a chart with CEL type errors, and an event whose payload does not match its declared type is replaced by <code>error.execution</code> before the chart sees it."),
            ("ul", [
                "CEL-level errors, such as an operator applied to the wrong types, are reported only after the structural checks pass, and without a line and column.",
                "<code>vt schema --format rust</code> is listed in <code>--help</code> but exits with code 2: Rust types come from the <code>statechart!</code> macro.",
            ]),
        ],
    },
    {
        "slug": "rust-macro",
        "title": "Rust macro",
        "num": "III",
        "summary": "A turnstile written with statechart!, with guards and actions as ordinary Rust functions.",
        "lede": "A coin-operated turnstile written with the <code>statechart!</code> macro on the <code>rust</code> datamodel. The macro checks the chart when the program compiles and generates the context, event and state types, the chart version and a <code>Machine</code>.",
        "files": ["rust-macro/main.rs", "rust-macro/expected-output.txt"],
        "body": lambda: [
            ("h2", "The program"),
            ("p", "Guards are <code>fn(&amp;Ctx, &amp;P) -&gt; bool</code> and actions are <code>fn(&amp;mut Ctx, &amp;P, &amp;mut Emit)</code> or <code>fn(&amp;mut Ctx, &amp;P)</code>, bound by name. A targetless <code>on coin / add_credit</code> counts coins without leaving the state, and an eventless <code>always if paid =&gt; unlocked</code> opens the turnstile once there is enough credit."),
            ("code", (read("rust-macro/main.rs"), "Rust")),
            ("h2", "Run it"),
            ("raw", run("cargo run -p vertep-examples --bin rust-macro")),
            ("out", (read("rust-macro/expected-output.txt"), "Output")),
            ("p", "<code>off</code> is a top-level final state, so <code>Shutdown</code> ends the session and the configuration is empty. That is standard SCXML."),
        ],
    },
    {
        "slug": "durable-actor",
        "title": "Durable actor",
        "num": "IV",
        "summary": "A virtual actor on a tiramemsu journal: the process stops, a new one replays it, and a durable timer fires.",
        "lede": "A virtual actor on a tiramemsu store. The first process sends three events and stops. A second process opens the same directory, replays the journal, and the timer the first one scheduled fires in the second as an ordinary journaled input.",
        "files": ["durable-actor/order.scxml", "durable-actor/main.rs", "durable-actor/expected-output.txt"],
        "body": lambda: [
            ("h2", "The chart"),
            ("p", "<code>awaiting_payment</code> schedules <code>payment.timeout</code> 900 seconds ahead. The outcome states are plain states rather than top-level finals, so the final state stays visible in the output."),
            ("code", (read("durable-actor/order.scxml"), "SCXML")),
            ("h2", "The program"),
            ("p", "Both processes run on a virtual clock, so the output is stable. A server runs the same node on the system clock with <code>Shell::tokio()</code> and <code>node.run().await</code>; <code>crates/vertep/examples/quickstart.rs</code> shows that."),
            ("code", (read("durable-actor/main.rs"), "Rust")),
            ("h2", "Run it"),
            ("raw", run("cargo run -p vertep-examples --bin durable-actor")),
            ("out", (read("durable-actor/expected-output.txt"), "Output")),
            ("ul", [
                "Every accepted input is appended to the journal before the actor processes it, so the second process rebuilds the state by replay alone.",
                "The timer is journaled with the entry that scheduled it, so it survives the stop and fires once.",
                "<code>state_at</code> reads the actor as of an earlier entry: time travel by replay.",
            ]),
        ],
    },
    {
        "slug": "decider-agent",
        "title": "Decider agent",
        "num": "V",
        "summary": "Two vt:decide choice points answered by a scripted decider, then replayed with no decider at all.",
        "lede": "A refund request with two choice points. A scripted decider stands in for a language model, so the example needs no network and no API key. The chart decides what may be chosen; the decider only picks.",
        "files": ["decider-agent/refund.scxml", "decider-agent/main.rs", "decider-agent/expected-output.txt"],
        "body": lambda: [
            ("h2", "The chart"),
            ("p", "<code>vt:decide=\"policy\"</code> hands the choice among a state's events to whatever decider the host binds to <code>policy</code>. The chart still owns the rules. With an amount of 250, the guard <code>ctx.amount &lt;= 100</code> keeps <code>approve</code> out of the offer in <code>review</code>."),
            ("code", (read("decider-agent/refund.scxml"), "SCXML")),
            ("h2", "The program"),
            ("p", "The first <code>AgentHost</code> binds the scripted decider and runs the request. The second opens the same journal with no decider bound and replays it."),
            ("code", (read("decider-agent/main.rs"), "Rust")),
            ("h2", "Run it"),
            ("raw", run("cargo run -p vertep-examples --bin decider-agent")),
            ("out", (read("decider-agent/expected-output.txt"), "Output")),
            ("ul", [
                "The decider was offered <code>reject</code> and <code>escalate</code> in <code>review</code>. <code>approve</code> was never on the table.",
                "Each decision is a journal entry with its offer's correlation id and the decider's rationale.",
                "The replay reaches the same states with zero decider calls.",
            ]),
            ("nb", "For a language-model agent with tools and a fork from the journal, see <code>crates/vertep-agent/examples/react_support_agent.rs</code> and <code>fork_from_journal.rs</code>, and the article <a href=\"../articles/deterministic-agents.html\">The goat does not choose the play</a>."),
            ("warn", "The agent host keeps its own journal, and only the in-memory <code>MemoryJournal</code> implements it today. A durable agent journal on tiramemsu is not there yet."),
        ],
    },
    {
        "slug": "browser-js",
        "title": "Browser and Node.js",
        "num": "VI",
        "summary": "@vertep/web from Node.js: spawn from SCXML text, send, subscribe, read the journal, travel back.",
        "lede": "An ES module that uses <code>@vertep/web</code>, the package a browser loads. The actor system runs in a worker, a worker thread in Node.js, with the core compiled to WebAssembly.",
        "files": ["browser-js/main.mjs", "browser-js/package.json", "browser-js/expected-output.txt"],
        "body": lambda: [
            ("h2", "The program"),
            ("p", "It spawns a cart actor from SCXML text with in-memory storage, subscribes to snapshots, sends events as <code>{ type, ...payload }</code>, reads the journal with the transitions each entry took, and reads the state as of an earlier entry."),
            ("code", (read("browser-js/main.mjs"), "JavaScript")),
            ("h2", "Build once, then run"),
            ("p", "In your own project, <code>npm install @vertep/web</code> is all you need. This example runs against the checkout instead: its <code>package.json</code> links <code>@vertep/web</code> to the local build, so build the packages once first."),
            ("raw", run("rustup target add wasm32-unknown-unknown\ncargo install wasm-bindgen-cli --version 0.2.128\ncd bindings/js && npm install && npm run build\n\ncd ../../examples/browser-js\nnpm install     # links the local @vertep/web; no download\nnpm start")),
            ("out", (read("browser-js/expected-output.txt"), "Output")),
            ("ul", [
                "An event no transition takes, here <code>pay</code>, is still journaled but produces no snapshot.",
                "<code>subscribe</code> does not deliver the current snapshot, and notifications arrive asynchronously.",
                "In a browser, pass <code>storage: \"opfs\"</code> for a durable journal in the origin private file system.",
            ]),
        ],
    },
]


def ts_excerpt() -> str:
    lines = read("typed-order/vt/order.ts").splitlines()
    keep = []
    for line in lines:
        if line.startswith(("export type", "export interface", "  |", "  items", "  total", "  limit", "}")):
            keep.append(line)
        if line.startswith("export type OrderStateId"):
            break
    return "\n".join(keep)


def page(title: str, desc: str, current: str, sidebar: str, main: str) -> str:
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — Vertep examples</title>
<meta name="description" content="{escape(desc)}">
<link rel="icon" href="../assets/img/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="{FONTS}">
<link rel="stylesheet" href="../assets/css/vertep.css">
</head>
<body>
<a class="skip" href="#main">Skip to content</a>
<!-- Generated by site/tools/sync-examples.py from examples/; edit the script, not this file. -->

<header class="masthead">
  <div class="masthead-inner">
    <a class="brand" href="../index.html"><img src="../assets/img/logo.svg" alt="" width="46" height="46"><span class="wordmark">Vertep</span></a>
    <nav class="topnav" aria-label="Main">
      <a href="../docs/index.html">Docs</a>
      <a href="index.html" aria-current="page">Examples</a>
      <a href="../articles/index.html">Articles</a>
      <a href="../about.html">About</a>
    </nav>
  </div>
</header>

<div class="docs-layout">
{sidebar}
  <main id="main" class="content">
{main}
  </main>
</div>

<footer class="colophon">
  <div class="colophon-inner">
    <div class="mark"><img src="../assets/img/logo.svg" alt="" width="40" height="40">Vertep</div>
    <small>Named for the travelling puppet theatre of Ukraine. Licensed MIT or Apache-2.0.</small>
    <nav class="topnav" aria-label="Footer"><a href="../docs/index.html" style="color:inherit">Docs</a><a href="index.html" style="color:inherit">Examples</a><a href="../articles/index.html" style="color:inherit">Articles</a><a href="https://github.com/Volland/vertep" style="color:inherit">GitHub</a><a href="../about.html" style="color:inherit">About</a><a href="https://lean4knowledge.com/" style="color:inherit">Lean 4 Knowledge</a><a href="https://pavlyshyn.me/" style="color:inherit">pavlyshyn.me</a><a href="../impressum.html" style="color:inherit">Impressum</a><a href="../datenschutz.html" style="color:inherit">Datenschutz</a><a href="../agb.html" style="color:inherit">AGB</a></nav>
  </div>
</footer>
</body>
</html>
"""


def sidebar(current: str) -> str:
    def link(href: str, text: str, slug: str) -> str:
        cur = ' aria-current="page"' if slug == current else ""
        return f'      <li><a href="{href}"{cur}>{text}</a></li>'

    items = [link("index.html", "All examples", "index")]
    items += [link(f"{e['slug']}.html", f"{e['num']}. {e['title']}", e["slug"]) for e in EXAMPLES]
    return (
        '  <aside class="sidebar" aria-label="Examples">\n    <h4>Examples</h4>\n    <ul>\n'
        + "\n".join(items)
        + '\n    </ul>\n    <h4>Elsewhere</h4>\n    <ul>\n'
        '      <li><a href="../docs/index.html">Getting started</a></li>\n'
        '      <li><a href="../docs/cli.html">The vt command</a></li>\n'
        "    </ul>\n  </aside>"
    )


def render_block(kind: str, value) -> str:
    if kind == "p":
        return f"    <p>{value}</p>"
    if kind == "h2":
        anchor = value.lower().replace(" ", "-")
        return f'    <h2 id="{escape(anchor)}">{escape(value)}</h2>'
    if kind == "code":
        return code(*value)
    if kind == "out":
        return code(value[0], value[1], output=True)
    if kind == "raw":
        return value
    if kind == "ul":
        return "    <ul>\n" + "\n".join(f"      <li>{li}</li>" for li in value) + "\n    </ul>"
    if kind == "nb":
        return f'    <div class="nb"><p>{value}</p></div>'
    if kind == "warn":
        return f'    <div class="nb warn"><p>{value}</p></div>'
    raise ValueError(kind)


def example_page(i: int, e: dict) -> str:
    prev_link = '<a href="index.html">← All examples</a>' if i == 0 else f'<a href="{EXAMPLES[i-1]["slug"]}.html">← {EXAMPLES[i-1]["title"]}</a>'
    next_link = f'<a href="{EXAMPLES[i+1]["slug"]}.html">{EXAMPLES[i+1]["title"]} →</a>' if i + 1 < len(EXAMPLES) else '<a href="../docs/index.html">Getting started →</a>'
    files = ", ".join(f"<code>examples/{escape(f)}</code>" for f in e["files"])
    blocks = "\n".join(render_block(k, v) for k, v in e["body"]())
    main = f"""    <p class="breadcrumb"><a href="index.html">Examples</a> · {e['num']}</p>
    <h1>{escape(e['title'])}</h1>
    <p class="lede">{e['lede']}</p>
    <p>Files: {files}. All commands run from the repository root.</p>
{blocks}
    <div class="ornament" aria-hidden="true"><img class="star-glyph" src="../assets/img/star.svg" alt=""></div>
    <nav class="pager" aria-label="Examples">
      {prev_link}
      {next_link}
    </nav>"""
    return page(e["title"], e["summary"], e["slug"], sidebar(e["slug"]), main)


def index_page() -> str:
    cards = "\n".join(
        f"""      <a class="card" href="{e['slug']}.html">
        <div class="card-head"><span class="num">{e['num']}</span><h3>{escape(e['title'])}</h3></div>
        <div class="card-body"><p>{escape(e['summary'])}</p><span class="more">Open →</span></div>
      </a>"""
        for e in EXAMPLES
    )
    main = f"""    <p class="breadcrumb">Examples</p>
    <h1>Examples</h1>
    <p class="lede">Six small programs, one idea each. They live in <code>examples/</code> in the repository, and <code>cargo test -p vertep-examples</code> runs every one and compares what it prints with the output shown on these pages.</p>
    <div class="cards">
{cards}
    </div>
    <h2 id="setup">Setup</h2>
    <p>The Rust examples need only the workspace toolchain. For the <code>vt</code> commands, install the CLI from the checkout, or alias it to cargo:</p>
{run("cargo install --path crates/vertep-cli        # puts `vt` on your PATH" + chr(10) + "# or" + chr(10) + "alias vt='cargo run -q -p vertep-cli --bin vt --'")}
    <p>The browser example needs the JavaScript packages built once; its page shows how.</p>
    <h2 id="honest">How they stay honest</h2>
    <ul>
      <li><code>examples/tests/run_examples.rs</code> runs each binary and compares its output with the <code>expected-output.txt</code> next to it. It also runs the Node.js example when Node and its <code>node_modules</code> are present.</li>
      <li><code>examples/tests/vt_commands.rs</code> runs the <code>vt</code> commands and compares them with the recorded files under <code>vt/</code>.</li>
      <li>After an intended change, <code>VERTEP_BLESS=1 cargo test -p vertep-examples</code> re-records the outputs, and <code>python3 site/tools/sync-examples.py</code> rewrites these pages.</li>
    </ul>
    <nav class="pager" aria-label="Examples">
      <a href="../docs/index.html">← Getting started</a>
      <a href="{EXAMPLES[0]['slug']}.html">{EXAMPLES[0]['title']} →</a>
    </nav>"""
    return page("Examples", "Runnable Vertep examples with their real output.", "index", sidebar("index"), main)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "index.html").write_text(index_page())
    for i, e in enumerate(EXAMPLES):
        (OUT / f"{e['slug']}.html").write_text(example_page(i, e))
    print(f"wrote {1 + len(EXAMPLES)} pages to {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
