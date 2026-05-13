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

function escapeHtml(s) {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
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

function renderUnitDetail(payload) {
  const u = payload.unit || {};
  const links = payload.links || { incoming: [], outgoing: [] };
  const linkItems = [
    ...(links.outgoing || []).map((l) => ({ ...l, dir: "→" })),
    ...(links.incoming || []).map((l) => ({ ...l, dir: "←" })),
  ];
  const linksHtml =
    linkItems.length === 0
      ? `<li class="modal__link-empty">no links</li>`
      : linkItems
          .map((l) => {
            const other = l.dir === "→" ? l.to_id : l.from_id;
            return `<li><span class="modal__link-rel">${escapeHtml(l.relationship)}</span> ${escapeHtml(l.dir)} <code>${escapeHtml(shortId(other))}</code></li>`;
          })
          .join("");

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
      <div class="wiki__markdown">${renderedBody}</div>
      <h3 class="modal__section-title">Links</h3>
      <ul class="modal__links">${linksHtml}</ul>
    </article>
  `;
}

async function openUnit(id) {
  setStatus(`loading unit ${shortId(id)} …`);
  try {
    const payload = await fetchJson(`/api/units/${encodeURIComponent(id)}`);
    renderUnitDetail(payload);
    setStatus(`unit ${shortId(id)} ok — ${shortTime()}`, "ok");
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
    const groups = groupByType(units);
    renderSidebar(groups);
    setStatus(`${path} ok — ${units.length} units — ${shortTime()}`, "ok");
  } catch (err) {
    setStatus(`${path} failed: ${err.message}`, "error");
    els.sidebar.innerHTML = `<div class="wiki__sidebar-empty">failed: ${escapeHtml(err.message)}</div>`;
  }
}

// Track section expand/collapse so re-render preserves user state.
els.sidebar.addEventListener("toggle", (e) => {
  const det = e.target.closest("details.wiki__section");
  if (!det) return;
  const t = det.dataset.type;
  if (det.open) expanded.add(t);
  else expanded.delete(t);
}, true);

els.sidebar.addEventListener("click", (e) => {
  const btn = e.target.closest("button.wiki__entry");
  if (!btn) return;
  const id = btn.dataset.id;
  // Visual selection.
  for (const el of els.sidebar.querySelectorAll(".wiki__entry--active")) {
    el.classList.remove("wiki__entry--active");
  }
  btn.classList.add("wiki__entry--active");
  openUnit(id);
});

els.includeArchived.addEventListener("change", loadUnits);

loadUnits();
