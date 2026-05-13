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

  els.content.innerHTML = `
    <article class="wiki__detail">
      <header class="wiki__detail-header">
        <span class="badge badge--${escapeHtml(u.type || "")}">${escapeHtml(u.type || "")}</span>
        <code class="wiki__detail-id">${escapeHtml(u.id || "")}</code>
        ${u.archived ? `<span class="wiki__entry-tag">archived</span>` : ""}
      </header>
      <dl class="modal__meta">
        <dt>slug</dt><dd>${u.slug ? `<code>${escapeHtml(u.slug)}</code>` : "—"}</dd>
        <dt>tags</dt><dd>${escapeHtml(fmtTags(u.tags)) || "—"}</dd>
        <dt>source</dt><dd>${escapeHtml(u.source || "—")}</dd>
        <dt>confidence</dt><dd>${escapeHtml(fmtConfidence(u.confidence))}</dd>
        <dt>updated</dt><dd>${escapeHtml(u.updated || "—")}</dd>
      </dl>
      <h3 class="modal__section-title">Content</h3>
      <pre class="modal__content">${escapeHtml(u.content || "")}</pre>
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
