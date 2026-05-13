// Wiki view — browse units grouped by knowledge type.
//
// Loads all units via /api/search (which delegates to `simaris list --json`)
// and renders a collapsible sidebar with one section per type. Units inside
// each section retain the CLI's `created DESC` ordering, which approximates
// "most-recently-updated first" — edits don't bump position today; if the
// CLI later surfaces `updated`, swap the sort. Archived (and superseded)
// units are hidden by default; the toggle re-fetches with
// `include_archived=1` so the server returns archived rows too. Superseded
// detection is not yet plumbed through the lean list output — the toggle
// label reflects the implemented behaviour.
//
// Reader pane renders unit content as markdown via marked (loaded from CDN
// in wiki.html). Headline is parsed from the first `# heading` line of the
// body; the headline line itself is stripped from the rendered body so it
// appears once in the header rather than duplicated. Per story 3, only the
// rendered output is shown — raw markdown belongs to edit mode (story 4).
//
// Cross-links (story alvp): after markdown render, we walk text nodes
// (skipping <code>/<pre>/<a> subtrees) and rewrite three patterns into
// anchors that navigate inside the wiki without a full page reload:
//   - bare UUIDv7 strings → <a data-id=...>
//   - recognized slugs (membership in the unit list payload) → <a data-id=...>
//   - [[wiki-syntax]] tokens resolving to a slug or id → <a data-id=...>
//     or, if the target isn't a known slug/id, a visually distinct broken
//     span.
// The right side of the reader pane shows two panels:
//   - "Links" — outgoing relationships grouped by relationship type
//   - "Backlinks" — every unit pointing at the current one, flat list,
//     labeled by the source unit's headline
// All anchors and panel entries route through openUnit(), which pushes
// `?unit=<id>` to history. popstate reopens the corresponding unit so
// browser back/forward work naturally.

import { fetchJson, setStatus, shortTime } from "./app.js";

const TYPE_ORDER = [
  "fact",
  "procedure",
  "principle",
  "preference",
  "lesson",
  "idea",
  "aspect",
];

const els = {
  includeArchived: document.getElementById("wiki-include-archived"),
  sidebar: document.getElementById("wiki-sidebar-body"),
  content: document.getElementById("wiki-content"),
};

// Track which sections are expanded across re-renders so toggling the
// archived switch doesn't collapse the user's open sections.
const expanded = new Set();

// Resolution tables populated by loadUnits(). slugToId is used both by the
// cross-link rewriter and by [[wiki-syntax]] targets; idSet lets the
// rewriter validate raw UUIDs that appear in content.
const slugToId = new Map();
const idSet = new Set();
// Cache of id → unit-list summary so cross-links can resolve to a headline
// without an extra round-trip per render.
const unitsById = new Map();

// Last unit id rendered, so we don't push duplicate history entries when a
// click re-navigates to the same target.
let currentUnitId = null;

function escapeHtml(s) {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function escapeRegex(s) {
  return String(s).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function shortId(id) {
  if (!id) return "";
  return id.split("-")[0];
}

// Headline → slug → id prefix, per spec.
function entryLabel(u) {
  if (u.snippet && u.snippet.trim().length > 0) return u.snippet.trim();
  if (u.slug) return u.slug;
  return shortId(u.id);
}

// Best-effort headline for a unit referenced by a cross-link. Falls back
// through slug → short id when the unit isn't in our list cache (e.g.
// referenced unit is archived while the toggle is off).
function headlineFor(id) {
  const u = unitsById.get(id);
  if (u) return entryLabel(u);
  return shortId(id);
}

function groupByType(units) {
  const groups = new Map();
  for (const u of units) {
    const t = u.type || "(unknown)";
    if (!groups.has(t)) groups.set(t, []);
    groups.get(t).push(u);
  }
  // Stable order: canonical TYPE_ORDER first, then any extras alphabetically.
  const seen = new Set();
  const ordered = [];
  for (const t of TYPE_ORDER) {
    if (groups.has(t)) {
      ordered.push([t, groups.get(t)]);
      seen.add(t);
    }
  }
  for (const t of [...groups.keys()].filter((k) => !seen.has(k)).sort()) {
    ordered.push([t, groups.get(t)]);
  }
  return ordered;
}

function renderSidebar(groups) {
  if (groups.length === 0) {
    els.sidebar.innerHTML = `<div class="wiki__sidebar-empty">no units</div>`;
    return;
  }
  const parts = [];
  for (const [type, units] of groups) {
    const isOpen = expanded.has(type);
    parts.push(`
      <details class="wiki__section" data-type="${escapeHtml(type)}"${isOpen ? " open" : ""}>
        <summary class="wiki__section-summary">
          <span class="badge badge--${escapeHtml(type)}">${escapeHtml(type)}</span>
          <span class="wiki__section-count">${units.length}</span>
        </summary>
        <ul class="wiki__list">
          ${units
            .map(
              (u) => `
            <li class="wiki__item${u.archived ? " wiki__item--archived" : ""}" data-id="${escapeHtml(u.id)}">
              <button type="button" class="wiki__entry" data-id="${escapeHtml(u.id)}">
                <span class="wiki__entry-label">${escapeHtml(entryLabel(u))}</span>
                ${u.archived ? `<span class="wiki__entry-tag">archived</span>` : ""}
              </button>
            </li>
          `,
            )
            .join("")}
        </ul>
      </details>
    `);
  }
  els.sidebar.innerHTML = parts.join("");
}

function fmtTags(tags) {
  if (!Array.isArray(tags) || tags.length === 0) return "";
  return tags.join(", ");
}

function fmtConfidence(c) {
  if (typeof c !== "number") return "—";
  return c.toFixed(2);
}

// Pull `# heading` (level-1) from the first non-blank line of the body so the
// reader pane can show it prominently. Returns { headline, body } where body
// has the heading line removed if it matched (avoids duplicating the title in
// the rendered content). Falls back to no headline if the first line isn't a
// level-1 heading — caller decides what to substitute (slug, id).
function splitHeadline(content) {
  if (!content) return { headline: "", body: "" };
  const lines = content.split("\n");
  let i = 0;
  while (i < lines.length && lines[i].trim() === "") i += 1;
  if (i >= lines.length) return { headline: "", body: content };
  const m = /^#\s+(.+?)\s*$/.exec(lines[i]);
  if (!m) return { headline: "", body: content };
  const headline = m[1].trim();
  const rest = lines.slice(i + 1);
  // Drop one trailing blank line so the body doesn't start with a vacant gap.
  if (rest.length && rest[0].trim() === "") rest.shift();
  return { headline, body: rest.join("\n") };
}

// Render markdown to HTML via the marked CDN script. Configured GFM-on,
// breaks-off (paragraph behaviour matches CommonMark, not Slack/Discord).
// marked escapes HTML in plain text by default; we accept the residual risk
// of inline raw HTML because the wiki only renders the user's own local
// knowledge base — this admin UI is a single-user, local-loopback surface.
function renderMarkdown(md) {
  if (!md) return "";
  if (typeof window.marked === "undefined") {
    // CDN script failed to load — degrade to escaped plaintext rather than
    // crash the reader pane.
    return `<pre class="wiki__markdown-fallback">${escapeHtml(md)}</pre>`;
  }
  return window.marked.parse(md, { gfm: true, breaks: false });
}

// UUIDv7 wire format. Match permissively — any 8-4-4-4-12 hex string with
// hyphens is treated as a unit id. If the id isn't in our local map the
// click will hit /api/units/:id and the server will return the actual unit
// or a 404-style error in the reader pane.
const UUID_PATTERN_SRC =
  "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";

// Build the cross-link scanner: three alternations, one capture group each.
// Group order is preserved across alternations so we can dispatch in
// rewriteTextNode().
//   group 1 — [[wiki-syntax]] target text (no brackets)
//   group 2 — bare UUIDv7
//   group 3 — recognized slug token
// We rebuild the regex per slug-set so additions don't need a page reload
// fallback.
function buildXrefRegex(slugs) {
  const wikiPart = "\\[\\[([^\\[\\]]+)\\]\\]";
  const uuidPart = `(${UUID_PATTERN_SRC})`;
  let regexSrc = `${wikiPart}|${uuidPart}`;
  if (slugs.length > 0) {
    // Longest-first ordering so prefix-only slugs (e.g. "vox") don't
    // shadow more specific ones (e.g. "vox-bridge").
    const escaped = [...slugs]
      .sort((a, b) => b.length - a.length)
      .map(escapeRegex)
      .join("|");
    const slugPart = `\\b(${escaped})\\b`;
    regexSrc = `${regexSrc}|${slugPart}`;
  }
  return new RegExp(regexSrc, "gi");
}

// Avoid rewriting text inside these elements (code blocks render literal
// strings; existing <a> anchors already navigate; wiki__xref anchors we
// generated should not be reprocessed).
const SKIP_ANCESTOR_TAGS = new Set(["CODE", "PRE", "A"]);

function hasSkippableAncestor(node, rootEl) {
  let cur = node.parentNode;
  while (cur && cur !== rootEl) {
    if (cur.nodeType === 1 && SKIP_ANCESTOR_TAGS.has(cur.tagName)) {
      return true;
    }
    cur = cur.parentNode;
  }
  return false;
}

// Returns an HTML string fragment for a cross-link anchor. `target` is the
// resolved unit id (or null for a broken wiki-syntax link). `display` is
// the user-visible label. `kind` distinguishes 'uuid' | 'slug' | 'wiki' |
// 'broken' so CSS can style each variant.
function xrefAnchorHtml(kind, target, display) {
  if (kind === "broken") {
    return `<span class="wiki__xref wiki__xref--broken" title="unknown target">${escapeHtml(display)}</span>`;
  }
  return `<a href="?unit=${encodeURIComponent(target)}" class="wiki__xref wiki__xref--${escapeHtml(kind)}" data-id="${escapeHtml(target)}">${escapeHtml(display)}</a>`;
}

// Replace cross-link patterns in a single text node by inserting a
// document fragment. Caller decides whether the node's ancestor chain is
// safe to rewrite.
function rewriteTextNode(node, xrefRegex) {
  const text = node.nodeValue;
  if (!text) return;
  xrefRegex.lastIndex = 0;
  let match;
  let lastIndex = 0;
  let html = "";
  let anyMatch = false;
  while ((match = xrefRegex.exec(text)) !== null) {
    anyMatch = true;
    const [, wikiTarget, uuidMatch, slugMatch] = match;
    const start = match.index;
    if (start > lastIndex) {
      html += escapeHtml(text.slice(lastIndex, start));
    }
    if (wikiTarget !== undefined) {
      // [[X]]: resolve X as slug, then as bare id. Unknown → broken span.
      const t = wikiTarget.trim();
      const id = slugToId.get(t) || (idSet.has(t.toLowerCase()) ? t.toLowerCase() : null);
      if (id) {
        html += xrefAnchorHtml("wiki", id, t);
      } else {
        html += xrefAnchorHtml("broken", null, t);
      }
    } else if (uuidMatch !== undefined) {
      const id = uuidMatch.toLowerCase();
      html += xrefAnchorHtml("uuid", id, uuidMatch);
    } else if (slugMatch !== undefined) {
      const id = slugToId.get(slugMatch);
      if (id) {
        html += xrefAnchorHtml("slug", id, slugMatch);
      } else {
        html += escapeHtml(slugMatch);
      }
    }
    lastIndex = start + match[0].length;
  }
  if (!anyMatch) return;
  if (lastIndex < text.length) {
    html += escapeHtml(text.slice(lastIndex));
  }
  const wrap = document.createElement("span");
  wrap.innerHTML = html;
  const frag = document.createDocumentFragment();
  while (wrap.firstChild) frag.appendChild(wrap.firstChild);
  node.parentNode.replaceChild(frag, node);
}

// Walk the rendered markdown subtree once, rewriting text-node content
// into cross-link anchors where applicable. TreeWalker collects nodes up
// front so the live mutations during rewrite don't disturb iteration.
function applyCrossLinks(rootEl) {
  if (!rootEl) return;
  const xrefRegex = buildXrefRegex([...slugToId.keys()]);
  // No slugs, no ids, no [[ ]] in body short-circuits when regex can't
  // match anything — but slugs may be empty while UUIDs/wiki-syntax still
  // need scanning, so we always run.
  const walker = document.createTreeWalker(rootEl, NodeFilter.SHOW_TEXT, null);
  const nodes = [];
  while (walker.nextNode()) {
    const n = walker.currentNode;
    if (!hasSkippableAncestor(n, rootEl)) nodes.push(n);
  }
  for (const n of nodes) rewriteTextNode(n, xrefRegex);
}

// Outgoing links grouped by relationship type — relationship name first
// (related_to, supersedes, depends_on, …) followed by each target entry
// labelled with its headline.
function renderLinksPanel(outgoing) {
  if (!outgoing || outgoing.length === 0) {
    return `<p class="wiki__panel-empty">no outgoing links</p>`;
  }
  const groups = new Map();
  for (const l of outgoing) {
    const rel = l.relationship || "(unknown)";
    if (!groups.has(rel)) groups.set(rel, []);
    groups.get(rel).push(l);
  }
  const relOrder = [...groups.keys()].sort();
  return relOrder
    .map((rel) => {
      const items = groups
        .get(rel)
        .map((l) => {
          const headline = l.headline || headlineFor(l.to_id);
          return `
            <li class="wiki__panel-item">
              <a href="?unit=${encodeURIComponent(l.to_id)}" class="wiki__panel-entry" data-id="${escapeHtml(l.to_id)}">
                <span class="wiki__panel-headline">${escapeHtml(headline)}</span>
                <span class="wiki__panel-type">${escapeHtml(l.type || "")}</span>
              </a>
            </li>
          `;
        })
        .join("");
      return `
        <div class="wiki__panel-group">
          <h4 class="wiki__panel-rel">${escapeHtml(rel)}</h4>
          <ul class="wiki__panel-list">${items}</ul>
        </div>
      `;
    })
    .join("");
}

// Backlinks: flat list of every unit pointing at the current one. AC says
// "via any relationship" — we surface the relationship name as secondary
// metadata rather than grouping by it.
function renderBacklinksPanel(incoming) {
  if (!incoming || incoming.length === 0) {
    return `<p class="wiki__panel-empty">no backlinks</p>`;
  }
  // Sort by relationship then headline for stable display.
  const sorted = [...incoming].sort((a, b) => {
    const ra = a.relationship || "";
    const rb = b.relationship || "";
    if (ra !== rb) return ra.localeCompare(rb);
    return (a.headline || "").localeCompare(b.headline || "");
  });
  const items = sorted
    .map((l) => {
      const headline = l.headline || headlineFor(l.from_id);
      return `
        <li class="wiki__panel-item">
          <a href="?unit=${encodeURIComponent(l.from_id)}" class="wiki__panel-entry" data-id="${escapeHtml(l.from_id)}">
            <span class="wiki__panel-headline">${escapeHtml(headline)}</span>
            <span class="wiki__panel-rel-inline">${escapeHtml(l.relationship || "")}</span>
            <span class="wiki__panel-type">${escapeHtml(l.type || "")}</span>
          </a>
        </li>
      `;
    })
    .join("");
  return `<ul class="wiki__panel-list">${items}</ul>`;
}

function renderUnitDetail(payload) {
  const u = payload.unit || {};
  const links = payload.links || { incoming: [], outgoing: [] };

  const { headline, body } = splitHeadline(u.content || "");
  const titleText = headline || u.slug || shortId(u.id);
  const renderedBody = renderMarkdown(body);

  els.content.innerHTML = `
    <article class="wiki__detail">
      <header class="wiki__detail-header">
        <h1 class="wiki__detail-title">${escapeHtml(titleText)}</h1>
        <div class="wiki__detail-meta-row">
          <span class="badge badge--${escapeHtml(u.type || "")}">${escapeHtml(u.type || "")}</span>
          <code class="wiki__detail-id">${escapeHtml(u.id || "")}</code>
          ${u.archived ? `<span class="wiki__entry-tag">archived</span>` : ""}
        </div>
      </header>
      <dl class="modal__meta">
        <dt>slug</dt><dd>${u.slug ? `<code>${escapeHtml(u.slug)}</code>` : "—"}</dd>
        <dt>tags</dt><dd>${escapeHtml(fmtTags(u.tags)) || "—"}</dd>
        <dt>source</dt><dd>${escapeHtml(u.source || "—")}</dd>
        <dt>confidence</dt><dd>${escapeHtml(fmtConfidence(u.confidence))}</dd>
        <dt>updated</dt><dd>${escapeHtml(u.updated || "—")}</dd>
      </dl>
      <div class="wiki__markdown" id="wiki-rendered-body">${renderedBody}</div>

      <section class="wiki__panel wiki__panel--links" aria-label="outgoing links">
        <h3 class="wiki__panel-title">Links</h3>
        ${renderLinksPanel(links.outgoing)}
      </section>

      <section class="wiki__panel wiki__panel--backlinks" aria-label="backlinks">
        <h3 class="wiki__panel-title">Backlinks</h3>
        ${renderBacklinksPanel(links.incoming)}
      </section>
    </article>
  `;

  // Post-process the markdown body to turn bare ids, recognized slugs, and
  // [[wiki-syntax]] tokens into wiki cross-links. The Links/Backlinks
  // panels are already anchors so we don't rewrite them.
  const bodyEl = els.content.querySelector("#wiki-rendered-body");
  applyCrossLinks(bodyEl);
}

// Reflect the active unit in the sidebar even when navigation came from a
// cross-link rather than a sidebar click.
function syncSidebarSelection(id) {
  for (const el of els.sidebar.querySelectorAll(".wiki__entry--active")) {
    el.classList.remove("wiki__entry--active");
  }
  if (!id) return;
  const btn = els.sidebar.querySelector(`.wiki__entry[data-id="${CSS.escape(id)}"]`);
  if (btn) btn.classList.add("wiki__entry--active");
}

async function openUnit(id, { push = true } = {}) {
  if (!id) return;
  setStatus(`loading unit ${shortId(id)} …`);
  try {
    const payload = await fetchJson(`/api/units/${encodeURIComponent(id)}`);
    renderUnitDetail(payload);
    setStatus(`unit ${shortId(id)} ok — ${shortTime()}`, "ok");
    syncSidebarSelection(id);
    if (push && id !== currentUnitId) {
      const url = `?unit=${encodeURIComponent(id)}`;
      history.pushState({ unit: id }, "", url);
    }
    currentUnitId = id;
  } catch (err) {
    setStatus(`unit ${shortId(id)} failed: ${err.message}`, "error");
    els.content.innerHTML = `<div class="placeholder">failed to load: ${escapeHtml(err.message)}</div>`;
  }
}

async function loadUnits() {
  const includeArchived = els.includeArchived.checked ? "1" : "";
  const params = new URLSearchParams();
  if (includeArchived) params.set("include_archived", includeArchived);
  const path = `/api/search${params.toString() ? `?${params.toString()}` : ""}`;
  setStatus(`loading wiki …`);
  try {
    const payload = await fetchJson(path);
    const units = (payload && payload.units) || [];
    // Rebuild resolution tables from the freshly fetched list.
    slugToId.clear();
    idSet.clear();
    unitsById.clear();
    for (const u of units) {
      if (u.slug) slugToId.set(u.slug, u.id);
      if (u.id) {
        idSet.add(String(u.id).toLowerCase());
        unitsById.set(u.id, u);
      }
    }
    const groups = groupByType(units);
    renderSidebar(groups);
    if (currentUnitId) syncSidebarSelection(currentUnitId);
    setStatus(`${path} ok — ${units.length} units — ${shortTime()}`, "ok");
  } catch (err) {
    setStatus(`${path} failed: ${err.message}`, "error");
    els.sidebar.innerHTML = `<div class="wiki__sidebar-empty">failed: ${escapeHtml(err.message)}</div>`;
  }
}

// Track section expand/collapse so re-render preserves user state.
els.sidebar.addEventListener(
  "toggle",
  (e) => {
    const det = e.target.closest("details.wiki__section");
    if (!det) return;
    const t = det.dataset.type;
    if (det.open) expanded.add(t);
    else expanded.delete(t);
  },
  true,
);

els.sidebar.addEventListener("click", (e) => {
  const btn = e.target.closest("button.wiki__entry");
  if (!btn) return;
  const id = btn.dataset.id;
  openUnit(id);
});

// Single delegated handler for every cross-link / panel-entry inside the
// reader pane. Anchors carry `data-id` so we don't have to parse hrefs.
// We intercept the click so the URL bar updates without a navigation.
els.content.addEventListener("click", (e) => {
  const anchor = e.target.closest("a[data-id]");
  if (!anchor) return;
  // Allow modifier-clicks to behave like normal anchors (open in new tab).
  if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  e.preventDefault();
  const id = anchor.dataset.id;
  if (!id) return;
  openUnit(id);
});

window.addEventListener("popstate", (e) => {
  const stateId = e.state && e.state.unit;
  const queryId = new URLSearchParams(window.location.search).get("unit");
  const id = stateId || queryId;
  if (id) {
    openUnit(id, { push: false });
  } else {
    els.content.innerHTML = `<div class="placeholder">select a unit from the sidebar to view details.</div>`;
    currentUnitId = null;
    syncSidebarSelection(null);
  }
});

els.includeArchived.addEventListener("change", loadUnits);

(async function init() {
  await loadUnits();
  // If the page was opened with ?unit=<id>, deep-link straight into the
  // reader pane. We use replaceState (push=false) so the back stack starts
  // clean rather than with a duplicate of the same target.
  const startId = new URLSearchParams(window.location.search).get("unit");
  if (startId) {
    await openUnit(startId, { push: false });
    history.replaceState({ unit: startId }, "", `?unit=${encodeURIComponent(startId)}`);
  }
})();
