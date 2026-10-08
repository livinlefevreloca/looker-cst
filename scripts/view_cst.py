"""Open a browsable view of a LookML file's concrete syntax tree.

Parses the file with looker_cst, writes a self-contained HTML page and opens it in the
browser. The tree starts expanded to --depth levels; click a node to drill into it and to see
its source, comments and leading whitespace in the side panel.

    uv run scripts/view_cst.py ../looker/models/analytics.model.lkml
    uv run scripts/view_cst.py some.view.lkml --depth 3 --out /tmp/tree.html --no-open
"""

import argparse
import bisect
import json
import sys
import tempfile
import webbrowser
from pathlib import Path
from typing import Any, Dict, List, Optional

import looker_cst
from looker_cst import Pair


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("file", type=Path, help="LookML file to view")
    parser.add_argument("--depth", type=int, default=2, help="levels expanded when the page opens (default 2)")
    parser.add_argument("--out", type=Path, help="where to write the HTML (default: a temp file)")
    parser.add_argument("--no-open", action="store_true", help="write the page without opening a browser")
    args = parser.parse_args()

    source = args.file.read_bytes().decode("utf-8")
    try:
        doc = looker_cst.parse(source)
    except looker_cst.LookmlSyntaxError as error:
        print(f"{args.file}: {error}", file=sys.stderr)
        return 1

    tree = TreeBuilder(source).document(doc)
    data = {"file": str(args.file), "source": source, "depth": args.depth, "tree": tree}
    # Escaping "</" keeps source text such as "</script>" from closing the embedded JSON block.
    payload = json.dumps(data, separators=(",", ":")).replace("</", "<\\/")
    html = PAGE.replace("__TITLE__", escape_html(args.file.name)).replace("__DATA__", payload)

    out = args.out or Path(tempfile.gettempdir()) / f"{args.file.name}.cst.html"
    out.write_text(html, encoding="utf-8")
    print(f"wrote {out}")
    if not args.no_open:
        webbrowser.open(out.resolve().as_uri())
    return 0


class TreeBuilder:
    """Turns the parsed document into plain nodes, locating each pair in the source by offset.

    Pairs print exactly as they were parsed and siblings are contiguous, so each pair's text is
    found at the end of its previous sibling, or just after its parent's opening line.
    """

    def __init__(self, source: str) -> None:
        self.source = source
        self.line_starts = [0] + [i + 1 for i, c in enumerate(source) if c == "\n"]

    def document(self, doc: looker_cst.Document) -> Dict[str, Any]:
        return {
            "kind": "document",
            "key": "document",
            "start": 0,
            "end": len(self.source),
            "line": 1,
            "endLine": self.line_of(len(self.source)),
            "children": self.pairs(doc.children, 0),
        }

    def pairs(self, pairs: List[Pair], cursor: int) -> List[Dict[str, Any]]:
        nodes = []
        for pair in pairs:
            node = self.pair(pair, cursor)
            nodes.append(node)
            cursor = node["end"]
        return nodes

    def pair(self, pair: Pair, cursor: int) -> Dict[str, Any]:
        full = pair.to_string(include_leading=True)
        start = self.source.find(full, cursor)
        if start < 0:
            raise RuntimeError(f"could not locate {pair!r} in the source")
        lead = len(full) - len(pair.to_string())
        key_start = start + lead
        end = start + len(full)
        node: Dict[str, Any] = {
            "kind": pair.kind,
            "key": pair.key,
            "name": pair.name,
            "leadStart": start,
            "start": key_start,
            "end": end,
            "line": self.line_of(key_start),
            "endLine": self.line_of(end - 1),
            "comments": pair.comments,
        }
        if pair.kind == "block":
            node["children"] = self.pairs(pair.children, key_start)
        elif pair.kind == "list":
            node["children"] = [list_node(element, i) for i, element in enumerate(pair.value)]
        else:
            node["value"] = pair.value
        return node

    def line_of(self, offset: int) -> int:
        return bisect.bisect_right(self.line_starts, offset)


def list_node(element: Any, index: int) -> Dict[str, Any]:
    """A list element: a scalar, a (key, value) pair as in filters, or a nested list."""
    if isinstance(element, tuple):
        key, value = element
        if isinstance(value, list):
            return {"kind": "item", "key": key, "children": [list_node(v, i) for i, v in enumerate(value)]}
        return {"kind": "item", "key": key, "value": value}
    if isinstance(element, list):
        return {"kind": "item", "key": f"[{index}]", "children": [list_node(v, i) for i, v in enumerate(element)]}
    return {"kind": "item", "key": f"[{index}]", "value": element}


def escape_html(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


PAGE = r"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>__TITLE__ · CST</title>
<style>
:root {
  --bg: #f7f7f5; --panel: #ffffff; --card: #ffffff; --text: #1d1d1b; --muted: #6b6b66;
  --line: #c9c9c2; --border: #deded8; --accent: #2f6fdf; --hit: #fff1a8; --hit-text: #1d1d1b;
  --ws: #b5b5ad; --code-bg: #f1f1ed;
  --k-block: #2f6fdf; --k-literal: #7b5bd6; --k-string: #2c8a4b; --k-expr: #c0602a;
  --k-list: #b4398a; --k-item: #6b6b66; --k-document: #1d1d1b;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #161615; --panel: #1f1f1d; --card: #242422; --text: #ececea; --muted: #9a9a94;
    --line: #4a4a46; --border: #34342f; --accent: #6d9cf0; --hit: #5c5012; --hit-text: #fff6c9;
    --ws: #5d5d57; --code-bg: #1a1a18;
    --k-block: #6d9cf0; --k-literal: #a68cf0; --k-string: #5cc07d; --k-expr: #e08a52;
    --k-list: #e06bb8; --k-item: #9a9a94; --k-document: #ececea;
  }
}
:root[data-theme="dark"] {
  --bg: #161615; --panel: #1f1f1d; --card: #242422; --text: #ececea; --muted: #9a9a94;
  --line: #4a4a46; --border: #34342f; --accent: #6d9cf0; --hit: #5c5012; --hit-text: #fff6c9;
  --ws: #5d5d57; --code-bg: #1a1a18;
  --k-block: #6d9cf0; --k-literal: #a68cf0; --k-string: #5cc07d; --k-expr: #e08a52;
  --k-list: #e06bb8; --k-item: #9a9a94; --k-document: #ececea;
}
* { box-sizing: border-box; }
html, body { height: 100%; margin: 0; }
body {
  background: var(--bg); color: var(--text);
  font: 13px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  display: flex; flex-direction: column; overflow: hidden;
}
code, pre, .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; }
header {
  display: flex; flex-wrap: wrap; gap: 8px 16px; align-items: center;
  padding: 10px 16px; border-bottom: 1px solid var(--border); background: var(--panel);
}
header h1 { font-size: 14px; margin: 0; font-weight: 600; overflow-wrap: anywhere; }
header .stats { color: var(--muted); }
.group { display: flex; gap: 4px; align-items: center; flex-wrap: wrap; }
.group > span { color: var(--muted); margin-right: 2px; }
button {
  font: inherit; color: var(--text); background: var(--card); border: 1px solid var(--border);
  border-radius: 6px; padding: 3px 9px; cursor: pointer;
}
button:hover { border-color: var(--accent); }
button.on { background: var(--accent); border-color: var(--accent); color: #fff; }
input[type=search] {
  font: inherit; color: var(--text); background: var(--card); border: 1px solid var(--border);
  border-radius: 6px; padding: 4px 8px; width: 220px; max-width: 100%;
}
#hits { color: var(--muted); min-width: 64px; }
main { flex: 1; display: flex; min-height: 0; }
#tree { flex: 1; overflow: auto; padding: 16px 16px 48px; min-width: 0; }
#detail {
  width: 440px; max-width: 45vw; border-left: 1px solid var(--border); background: var(--panel);
  overflow: auto; padding: 14px 16px; flex-shrink: 0;
}
@media (max-width: 760px) {
  main { flex-direction: column; }
  #detail { width: auto; max-width: none; height: 45vh; border-left: 0; border-top: 1px solid var(--border); }
}

/* Left to right tree: breadth stacks vertically, so only depth adds width. */
ul.rootlist { list-style: none; margin: 0; padding: 0; }
ul.kids { list-style: none; margin: 0; padding: 0 0 0 28px; position: relative; }
li.node { display: flex; align-items: flex-start; position: relative; padding: 3px 0; }
ul.kids > li.node::before {
  content: ""; position: absolute; left: -14px; top: 23px; width: 14px; border-top: 1px solid var(--line);
}
ul.kids > li.node::after {
  content: ""; position: absolute; left: -14px; top: 0; bottom: 0; border-left: 1px solid var(--line);
}
ul.kids > li.node:first-child::after { top: 23px; }
ul.kids > li.node:last-child::after { bottom: auto; height: 23px; }
ul.kids > li.node:only-child::after { display: none; }
ul.kids::before {
  content: ""; position: absolute; left: 0; top: 26px; width: 14px; border-top: 1px solid var(--line);
}
.card {
  width: 300px; flex-shrink: 0; background: var(--card); border: 1px solid var(--border);
  border-left: 3px solid var(--kind); border-radius: 6px; padding: 6px 9px; cursor: pointer;
  position: relative;
}
.card:hover { border-color: var(--accent); border-left-color: var(--kind); }
.card.selected { box-shadow: 0 0 0 2px var(--accent); }
.card.hit { background: var(--hit); color: var(--hit-text); }
.row { display: flex; gap: 6px; align-items: baseline; }
.badge {
  font-size: 10px; text-transform: uppercase; letter-spacing: .04em; color: var(--kind);
  font-weight: 600; flex-shrink: 0;
}
.title { font-weight: 600; overflow-wrap: break-word; flex: 1; min-width: 0; }
.title .name { color: var(--accent); font-weight: 500; }
.meta { color: var(--muted); font-size: 11px; flex-shrink: 0; white-space: nowrap; }
.preview {
  color: var(--muted); margin-top: 3px; overflow: hidden; display: -webkit-box;
  -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere; white-space: pre-wrap;
}
.card.hit .preview, .card.hit .meta { color: inherit; opacity: .8; }
.chev { color: var(--muted); width: 12px; display: inline-block; }
.more { padding: 3px 0; }
.more button { font-size: 12px; }

#detail h2 { font-size: 14px; margin: 0 0 4px; overflow-wrap: anywhere; }
#detail .sub { color: var(--muted); margin-bottom: 12px; }
#detail h3 { font-size: 11px; text-transform: uppercase; letter-spacing: .05em; color: var(--muted); margin: 16px 0 6px; }
#detail pre {
  background: var(--code-bg); border: 1px solid var(--border); border-radius: 6px; padding: 8px 10px;
  margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 50vh; overflow: auto;
}
.ws { color: var(--ws); }
.lead { background: color-mix(in srgb, var(--k-expr) 14%, transparent); }
.path { display: flex; flex-wrap: wrap; gap: 4px; margin-bottom: 10px; }
.path button { font-size: 11px; padding: 1px 6px; }
.empty { color: var(--muted); }
label.toggle { display: inline-flex; gap: 6px; align-items: center; color: var(--muted); cursor: pointer; }
</style>
</head>
<body>
<header>
  <h1 id="file"></h1>
  <span class="stats" id="stats"></span>
  <div class="group" id="depths"><span>Expand to depth</span></div>
  <div class="group">
    <input type="search" id="search" placeholder="Search keys, names, values" aria-label="Search">
    <span id="hits"></span>
  </div>
</header>
<main>
  <div id="tree"></div>
  <aside id="detail"><p class="empty">Click a node to see its source, comments and whitespace.</p></aside>
</main>
<script type="application/json" id="data">__DATA__</script>
<script>
"use strict";
const PAGE_SIZE = 20;
const data = JSON.parse(document.getElementById("data").textContent);
const source = data.source;
const root = data.tree;

// Index nodes and link parents so search can open the path to a match.
const all = [];
(function index(node, parent, depth) {
  node.id = all.length; node.parent = parent; node.depth = depth;
  node.expanded = false; node.shown = PAGE_SIZE;
  all.push(node);
  (node.children || []).forEach((child, i) => { child.index = i; index(child, node, depth + 1); });
})(root, null, 0);

let selected = null;
let hits = [];
let hitPos = -1;
const hitSet = new Set();
let showWhitespace = true;

document.getElementById("file").textContent = data.file;
document.getElementById("stats").textContent =
  `${(all.length - 1).toLocaleString()} nodes · ${root.endLine.toLocaleString()} lines`;

const treeEl = document.getElementById("tree");
const detailEl = document.getElementById("detail");

function hasKids(node) { return !!(node.children && node.children.length); }

function plural(n, one, many) { return `${n.toLocaleString()} ${n === 1 ? one : many}`; }

function preview(node) {
  const n = node.children ? node.children.length : 0;
  if (node.kind === "block") return n ? plural(n, "child", "children") : "empty block";
  if (node.kind === "document") return plural(n, "top level pair", "top level pairs");
  if (node.children) return `[${plural(n, "item", "items")}]`;
  return node.value == null ? "" : String(node.value);
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

// Long snake_case names wrap after an underscore or dot instead of mid-word.
function breakable(text, cls) {
  const span = el("span", cls);
  for (const part of text.split(/(?<=[_.])/)) {
    span.append(document.createTextNode(part));
    span.append(document.createElement("wbr"));
  }
  return span;
}

function renderNode(node) {
  const li = el("li", "node");
  li.dataset.id = node.id;
  const card = el("div", "card");
  card.style.setProperty("--kind", `var(--k-${node.kind})`);
  if (node === selected) card.classList.add("selected");
  if (hitSet.has(node.id)) card.classList.add("hit");

  const row = el("div", "row");
  row.append(el("span", "chev", hasKids(node) ? (node.expanded ? "▾" : "▸") : ""));
  row.append(el("span", "badge", node.kind));
  const title = el("span", "title");
  title.append(breakable(node.key));
  if (node.name) { title.append(document.createTextNode(": ")); title.append(breakable(node.name, "name")); }
  row.append(title);
  if (node.line) row.append(el("span", "meta", node.endLine > node.line ? `L${node.line}–${node.endLine}` : `L${node.line}`));
  card.append(row);
  const text = preview(node);
  if (text) {
    const p = el("div", "preview mono", text.length > 400 ? text.slice(0, 400) + "…" : text);
    card.append(p);
  }
  card.title = node.name ? `${node.key}: ${node.name}` : node.key;
  card.addEventListener("click", () => {
    if (hasKids(node)) node.expanded = !node.expanded;
    select(node, false);
    rerender(li, node);
  });
  li.append(card);
  if (node.expanded && hasKids(node)) li.append(renderKids(node));
  return li;
}

function renderKids(node) {
  const ul = el("ul", "kids");
  const count = Math.min(node.shown, node.children.length);
  for (let i = 0; i < count; i++) ul.append(renderNode(node.children[i]));
  const rest = node.children.length - count;
  if (rest > 0) {
    const li = el("li", "node more");
    const more = el("button", null, `Show ${Math.min(PAGE_SIZE, rest)} more`);
    more.addEventListener("click", () => { node.shown += PAGE_SIZE; replaceKids(ul, node); });
    const everything = el("button", null, `Show all ${node.children.length}`);
    everything.addEventListener("click", () => { node.shown = node.children.length; replaceKids(ul, node); });
    const label = el("span", "meta", ` ${rest} hidden `);
    li.append(more, label, everything);
    ul.append(li);
  }
  return ul;
}

function replaceKids(ul, node) { ul.replaceWith(renderKids(node)); }

function rerender(li, node) { li.replaceWith(renderNode(node)); }

function renderAll() {
  const scroll = [treeEl.scrollLeft, treeEl.scrollTop];
  const list = el("ul", "rootlist");
  list.append(renderNode(root));
  treeEl.replaceChildren(list);
  [treeEl.scrollLeft, treeEl.scrollTop] = scroll;
}

function expandToDepth(depth) {
  for (const node of all) {
    node.expanded = node.depth < depth;
    node.shown = PAGE_SIZE;
  }
  renderAll();
}

// Shows whitespace as visible marks so leading trivia and indentation can be checked.
function renderText(text, target, cls) {
  const span = el("span", cls);
  if (!showWhitespace) { span.textContent = text; target.append(span); return; }
  const parts = text.split(/( +|\t+|\n)/);
  for (const part of parts) {
    if (!part) continue;
    if (part === "\n") { span.append(el("span", "ws", "↵")); span.append(document.createTextNode("\n")); }
    else if (/^ +$/.test(part)) span.append(el("span", "ws", "·".repeat(part.length)));
    else if (/^\t+$/.test(part)) span.append(el("span", "ws", "→   ".repeat(part.length)));
    else span.append(document.createTextNode(part));
  }
  target.append(span);
}

function select(node, scrollIntoView) {
  const previous = selected;
  selected = node;
  for (const card of treeEl.querySelectorAll(".card.selected")) card.classList.remove("selected");
  const li = treeEl.querySelector(`li.node[data-id="${node.id}"]`);
  if (li) {
    li.firstChild.classList.add("selected");
    if (scrollIntoView) li.firstChild.scrollIntoView({ block: "center", inline: "center", behavior: "smooth" });
  }
  showDetail(node);
}

function showDetail(node) {
  detailEl.replaceChildren();
  const path = el("div", "path");
  const chain = [];
  for (let n = node; n; n = n.parent) chain.unshift(n);
  for (const n of chain) {
    const b = el("button", null, n.name ? `${n.key}: ${n.name}` : n.key);
    b.addEventListener("click", () => select(n, true));
    path.append(b);
  }
  detailEl.append(path);
  detailEl.append(el("h2", null, node.name ? `${node.key}: ${node.name}` : node.key));
  const facts = [node.kind];
  if (node.line) facts.push(node.endLine > node.line ? `lines ${node.line}–${node.endLine}` : `line ${node.line}`);
  if (node.children) {
    const isBlock = node.kind === "block" || node.kind === "document";
    facts.push(plural(node.children.length, isBlock ? "child" : "item", isBlock ? "children" : "items"));
  }
  detailEl.append(el("div", "sub", facts.join(" · ")));

  const toggle = el("label", "toggle");
  const box = Object.assign(document.createElement("input"), { type: "checkbox", checked: showWhitespace });
  box.addEventListener("change", () => { showWhitespace = box.checked; showDetail(node); });
  toggle.append(box, document.createTextNode("Show whitespace"));
  detailEl.append(toggle);

  if (node.comments && node.comments.length) {
    detailEl.append(el("h3", null, "Comments"));
    detailEl.append(el("pre", null, node.comments.join("\n")));
  }
  if (node.value != null && node.kind !== "block") {
    detailEl.append(el("h3", null, "Value"));
    const pre = el("pre");
    renderText(String(node.value), pre);
    detailEl.append(pre);
  }
  if (node.start != null && node.kind !== "document") {
    detailEl.append(el("h3", null, "Leading trivia"));
    const lead = source.slice(node.leadStart, node.start);
    const pre = el("pre");
    if (lead) renderText(lead, pre, "lead"); else pre.append(el("span", "empty", "(none)"));
    detailEl.append(pre);
  }
  if (node.start != null) {
    detailEl.append(el("h3", null, "Source"));
    const pre = el("pre");
    const text = source.slice(node.start, node.end);
    const limit = 200000;
    renderText(text.length > limit ? text.slice(0, limit) : text, pre);
    if (text.length > limit) pre.append(el("span", "empty", `\n… ${(text.length - limit).toLocaleString()} more characters`));
    detailEl.append(pre);
  }
}

// Search marks matching nodes and opens the path to each, then steps through them with Enter.
function search(query) {
  hitSet.clear();
  hits = [];
  hitPos = -1;
  const q = query.trim().toLowerCase();
  if (q) {
    for (const node of all) {
      const hay = [node.key, node.name, node.value == null ? "" : String(node.value)].join(" ").toLowerCase();
      if (node !== root && hay.includes(q)) { hits.push(node); hitSet.add(node.id); }
    }
  }
  document.getElementById("hits").textContent = q ? `${hits.length.toLocaleString()} found` : "";
  if (hits.length) nextHit(); else renderAll();
}

function reveal(node) {
  for (let n = node; n.parent; n = n.parent) {
    n.parent.expanded = true;
    if (n.index >= n.parent.shown) n.parent.shown = Math.ceil((n.index + 1) / PAGE_SIZE) * PAGE_SIZE;
  }
}

function nextHit() {
  if (!hits.length) return;
  hitPos = (hitPos + 1) % hits.length;
  reveal(hits[hitPos]);
  renderAll();
  select(hits[hitPos], true);
  document.getElementById("hits").textContent = `${hitPos + 1} of ${hits.length.toLocaleString()}`;
}

const depths = document.getElementById("depths");
for (const [label, depth] of [["1", 1], ["2", 2], ["3", 3], ["4", 4], ["All", Infinity]]) {
  const b = el("button", null, label);
  b.addEventListener("click", () => {
    for (const other of depths.querySelectorAll("button")) other.classList.remove("on");
    b.classList.add("on");
    expandToDepth(depth);
  });
  if (depth === data.depth) b.classList.add("on");
  depths.append(b);
}
const collapse = el("button", null, "Collapse");
collapse.addEventListener("click", () => {
  for (const other of depths.querySelectorAll("button")) other.classList.remove("on");
  expandToDepth(1);
});
depths.append(collapse);

let timer = null;
const searchEl = document.getElementById("search");
searchEl.addEventListener("input", () => { clearTimeout(timer); timer = setTimeout(() => search(searchEl.value), 200); });
searchEl.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); nextHit(); } });

expandToDepth(data.depth);
select(root, false);
</script>
</body>
</html>
"""


if __name__ == "__main__":
    sys.exit(main())
