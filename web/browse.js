// browse.js — two-pane card browser for simaris admin.
//
// Layout: search + filter chips (left top) → card list (left scroll) →
// detail pane (right). Keyboard-free, mouse-first. No modal.
//
// API:
//   GET  /api/search?q=          all units (list mode, created DESC)
//   GET  /api/search?q=<q>       FTS search
//   GET  /api/units/:id          { unit, links, slugs }
//   POST /api/units/:id          edit  { content, type, tags, source }
//   POST /api/units/:id/clone    clone
//   POST /api/units/:id/archive  archive
//   POST /api/units/:id/unarchive unarchive

import { fetchJson, setStatus } from "./app.js";

const PAGE_SIZE = 100;

const TYPES = [
  "fact", "procedure", "principle", "preference", "lesson", "idea", "aspect",
];

const TYPE_COL = {
  principle: "var(--c-principle)",
  fact:      "var(--c-fact)",
  lesson:    "var(--c-lesson)",
  procedure: "var(--c-procedure)",
  aspect:    "var(--c-aspect)",
  idea:      "var(--c-idea)",
  preference:"var(--c-preference)",
};

// ── State ──────────────────────────────────────────────────────────────────
let allUnits    = [];          // full list from /api/search?q=
let searchUnits = [];          // current result set (= allUnits when q empty)
let activeTypes = new Set(TYPES);
let searchQuery = "";
let visibleCount = PAGE_SIZE;
let selectedId   = null;
let editMode     = false;
let currentDetail = null;      // last /api/units/:id payload

// ── DOM ────────────────────────────────────────────────────────────────────
const elSearch   = document.getElementById("br-search");
const elChips    = document.getElementById("br-chips");
const elCardList = document.getElementById("br-card-list");
const elFooter   = document.getElementById("br-list-footer");
const elLoadMore = document.getElementById("br-load-more");
const elDetail   = document.getElementById("br-detail");
const elCount    = document.getElementById("br-count");

// ── Init ───────────────────────────────────────────────────────────────────
async function init() {
  setStatus("loading…");
  try {
    const data = await fetchJson("/api/search?q=");
    allUnits    = data.units ?? [];
    searchUnits = allUnits;
    renderChips();
    renderCards();
    elCount.textContent = `${allUnits.length} units`;
    setStatus(`${allUnits.length} units loaded.`);
    // Auto-select first card so detail pane isn't empty on load
    if (allUnits.length > 0) selectCard(allUnits[0].id);
  } catch (e) {
    setStatus(`load failed: ${e.message}`, "error");
    elCardList.innerHTML = `<div class="br-empty">failed to load — check server</div>`;
  }

  elSearch.addEventListener("input", debounce(onSearch, 280));
  elLoadMore.addEventListener("click", () => {
    visibleCount += PAGE_SIZE;
    renderCards();
  });
}

// ── Filter chips ───────────────────────────────────────────────────────────
function renderChips() {
  // Count by type across the full allUnits (not filtered) so counts don't
  // disappear when a chip is toggled off.
  const counts = {};
  for (const u of allUnits) counts[u.type] = (counts[u.type] ?? 0) + 1;

  elChips.innerHTML = TYPES
    .filter(t => counts[t] > 0)
    .map(t => {
      const on = activeTypes.has(t) ? " br-chip--on" : "";
      return `<button class="br-chip br-chip--${t}${on}" data-type="${t}">${t} <span class="br-chip-count">${counts[t]}</span></button>`;
    })
    .join("");

  elChips.querySelectorAll(".br-chip").forEach(btn =>
    btn.addEventListener("click", () => toggleType(btn.dataset.type))
  );
}

function toggleType(type) {
  if (activeTypes.has(type)) activeTypes.delete(type);
  else activeTypes.add(type);

  // Update chip appearance without full re-render
  elChips.querySelectorAll(".br-chip").forEach(btn =>
    btn.classList.toggle("br-chip--on", activeTypes.has(btn.dataset.type))
  );

  visibleCount = PAGE_SIZE;
  renderCards();
}

// ── Search ─────────────────────────────────────────────────────────────────
function onSearch() {
  const q = elSearch.value.trim();
  if (q === searchQuery) return;
  searchQuery = q;
  visibleCount = PAGE_SIZE;

  if (!q) {
    searchUnits = allUnits;
    renderCards();
    elCount.textContent = `${allUnits.length} units`;
    setStatus(`${allUnits.length} units.`);
  } else {
    doSearch(q);
  }
}

async function doSearch(q) {
  setStatus("searching…");
  try {
    const data = await fetchJson(`/api/search?q=${encodeURIComponent(q)}`);
    searchUnits = data.units ?? [];
    renderCards();
    const n = visibleUnits().length;
    elCount.textContent = `${n} results`;
    setStatus(`${n} results for "${q}".`);
  } catch (e) {
    setStatus(`search failed: ${e.message}`, "error");
  }
}

// ── Card list ──────────────────────────────────────────────────────────────
function visibleUnits() {
  return searchUnits.filter(u => activeTypes.has(u.type));
}

function renderCards() {
  const units = visibleUnits();
  if (units.length === 0) {
    elCardList.innerHTML = `<div class="br-empty">no results</div>`;
    elFooter.hidden = true;
    return;
  }

  const slice = units.slice(0, visibleCount);
  elCardList.innerHTML = slice.map(cardHTML).join("");
  elFooter.hidden = units.length <= visibleCount;

  // Event delegation via individual listeners (list is bounded by PAGE_SIZE)
  elCardList.querySelectorAll(".br-card").forEach(card =>
    card.addEventListener("click", () => selectCard(card.dataset.id))
  );

  // Restore selection highlight if card is still in view
  if (selectedId) {
    elCardList.querySelector(`.br-card[data-id="${CSS.escape(selectedId)}"]`)
      ?.classList.add("br-card--sel");
  }
}

function cardHTML(u) {
  const type = u.type ?? "unknown";
  const col  = TYPE_COL[type] ?? "var(--accent)";
  const conf = u.confidence != null ? Number(u.confidence).toFixed(2) : "—";
  const title   = cardTitle(u.snippet ?? "");
  const snippet = cardSnippet(u.snippet ?? "");
  const tags = (u.tags ?? []).slice(0, 4)
    .map(t => `<span class="br-card-tag">${esc(t)}</span>`).join("");
  const slug = u.slug ? `<div class="br-card-slug">${esc(u.slug)}</div>` : "";
  const sel  = u.id === selectedId ? " br-card--sel" : "";

  return `<div class="br-card br-card--${esc(type)}${sel}" data-id="${esc(u.id)}">
  <div class="br-card-header">
    <span class="br-card-dot" style="background:${col}"></span>
    <span class="br-card-type" style="color:${col}">${esc(type)}</span>
    <span class="br-card-conf">${esc(conf)}</span>
  </div>
  <div class="br-card-title">${esc(title)}</div>
  ${snippet ? `<div class="br-card-snippet">${esc(snippet)}</div>` : ""}
  ${tags    ? `<div class="br-card-tags">${tags}</div>` : ""}
  ${slug}
</div>`;
}

// Extract 1-line title from snippet (strip # heading marker).
function cardTitle(snippet) {
  if (!snippet) return "(empty)";
  const lines = snippet.split("\n").filter(l => l.trim());
  if (!lines.length) return "(empty)";
  return lines[0].replace(/^#+\s*/, "").trim() || snippet.trim().slice(0, 80);
}

// Extract second-line snippet for the card body.
function cardSnippet(snippet) {
  if (!snippet) return "";
  const lines = snippet.split("\n").filter(l => l.trim());
  if (lines.length <= 1) return "";
  return lines.slice(1).join(" ").replace(/^#+\s*/, "").trim().slice(0, 100);
}

// ── Select + detail ────────────────────────────────────────────────────────
async function selectCard(id) {
  if (id === selectedId && currentDetail && !editMode) return;
  selectedId = id;
  editMode   = false;

  // Update card highlight
  elCardList.querySelectorAll(".br-card").forEach(c =>
    c.classList.toggle("br-card--sel", c.dataset.id === id)
  );

  elDetail.innerHTML = `<div class="br-empty-state">loading…</div>`;

  try {
    const data = await fetchJson(`/api/units/${encodeURIComponent(id)}`);
    currentDetail = data;
    renderDetail(data);
  } catch (e) {
    elDetail.innerHTML = `<div class="br-empty-state">error: ${esc(e.message)}</div>`;
  }
}

function renderDetail(d) {
  const u    = d.unit   ?? {};
  const type = u.type   ?? "unknown";
  const col  = TYPE_COL[type] ?? "var(--accent)";
  const slugList = d.slugs ?? [];
  const slug = slugList[0] ?? null;

  const { headline, body } = splitHeadline(u.content ?? "");
  const title = headline || slug || shortId(u.id ?? "");
  const conf  = u.confidence != null ? Number(u.confidence).toFixed(2) : "—";
  const tags  = (u.tags ?? [])
    .map(t => `<span class="br-detail-tag">${esc(t)}</span>`).join("");
  const md = renderMarkdown(body || u.content || "");

  const outgoing = d.links?.outgoing ?? [];
  const incoming = d.links?.incoming ?? [];
  const linkCount = outgoing.length + incoming.length;

  const verifiedBadge = u.verified
    ? `<span style="font:10px var(--mono);color:var(--ok,#66bb6a)">✓ verified</span>`
    : "";
  const archivedBadge = u.archived
    ? `<span style="font:10px var(--mono);color:var(--error,#ef5350)">archived</span>`
    : "";

  elDetail.innerHTML = `
    <div class="br-detail-header">
      <div class="br-type-row">
        <div class="br-type-badge br-type-badge--${esc(type)}">
          <span class="br-dot"></span>${esc(type)}
        </div>
        ${slug            ? `<span class="br-detail-slug">${esc(slug)}</span>` : ""}
        <span class="br-detail-id">${esc(shortId(u.id ?? ""))}</span>
        <span class="br-detail-conf">conf ${esc(conf)}</span>
        ${verifiedBadge}${archivedBadge}
      </div>
      <div class="br-detail-title">${esc(title)}</div>
      <div class="br-detail-meta">
        ${u.updated ? `<span>updated ${esc(u.updated.slice(0, 10))}</span>` : ""}
        ${u.source  ? `<span>source: ${esc(u.source)}</span>`              : ""}
        ${u.byte_size != null ? `<span>${u.byte_size} B</span>`            : ""}
      </div>
      ${tags ? `<div class="br-detail-tags">${tags}</div>` : ""}
    </div>

    <div class="br-section-title">Content</div>
    <div class="br-content">${md}</div>

    <div class="br-section-title">Links (${linkCount})</div>
    ${linkCount > 0
      ? `<div class="br-link-list">
           ${outgoing.map(l => linkHTML(l, "→")).join("")}
           ${incoming.map(l => linkHTML(l, "←")).join("")}
         </div>`
      : `<p class="br-links-empty">no links</p>`
    }

    <div class="br-actions">
      <button class="br-btn br-btn--primary" id="br-edit-btn">Edit</button>
      <button class="br-btn" id="br-clone-btn">Clone</button>
      <button class="br-btn ${u.archived ? "" : "br-btn--danger"}" id="br-archive-btn">
        ${u.archived ? "Unarchive" : "Archive"}
      </button>
    </div>
  `;

  // Link item clicks → navigate detail pane to that unit
  elDetail.querySelectorAll(".br-link-item[data-id]").forEach(item =>
    item.addEventListener("click", () => selectCard(item.dataset.id))
  );

  document.getElementById("br-edit-btn")    ?.addEventListener("click", () => showEditForm(d));
  document.getElementById("br-clone-btn")   ?.addEventListener("click", () => doClone(u.id));
  document.getElementById("br-archive-btn") ?.addEventListener("click", () => doArchive(u.id, !!u.archived));
}

function linkHTML(link, dir) {
  const targetId = dir === "→" ? link.to_id : link.from_id;
  const text = link.headline ?? targetId ?? "";
  const ltype = link.type ?? "";
  return `<div class="br-link-item" data-id="${esc(targetId)}">
  <span class="br-link-rel">${esc(dir)} ${esc(link.relationship ?? "")}</span>
  <span class="br-link-text">${esc(text)}</span>
  <span class="br-link-type">${esc(ltype)}</span>
</div>`;
}

// ── Edit form ──────────────────────────────────────────────────────────────
function showEditForm(d) {
  editMode = true;
  const u    = d.unit ?? {};
  const tags = (u.tags ?? []).join(", ");

  // Keep header visible during edit; replace content below with form
  const headerHTML = elDetail.querySelector(".br-detail-header")?.outerHTML ?? "";

  elDetail.innerHTML = `
    ${headerHTML}
    <div class="br-edit-form" id="br-form">
      <div class="br-field">
        <label>Content</label>
        <textarea id="bf-content" rows="14">${esc(u.content ?? "")}</textarea>
      </div>
      <div class="br-form-row">
        <div class="br-field">
          <label>Type</label>
          <select id="bf-type">
            ${TYPES.map(t => `<option value="${t}"${t === u.type ? " selected" : ""}>${t}</option>`).join("")}
          </select>
        </div>
        <div class="br-field" style="flex:2">
          <label>Tags (comma-separated)</label>
          <input id="bf-tags" type="text" value="${esc(tags)}" />
        </div>
      </div>
      <div class="br-field">
        <label>Source</label>
        <input id="bf-source" type="text" value="${esc(u.source ?? "")}" />
      </div>
      <div class="br-actions">
        <button class="br-btn br-btn--primary" id="br-save-btn">Save</button>
        <button class="br-btn" id="br-cancel-btn">Cancel</button>
      </div>
    </div>
  `;

  document.getElementById("br-save-btn")   ?.addEventListener("click", () => doSave(u.id));
  document.getElementById("br-cancel-btn") ?.addEventListener("click", () => {
    editMode = false;
    renderDetail(currentDetail);
  });
}

async function doSave(id) {
  const content = document.getElementById("bf-content")?.value ?? "";
  const type    = document.getElementById("bf-type")?.value    ?? "";
  const tags    = document.getElementById("bf-tags")?.value    ?? "";
  const source  = document.getElementById("bf-source")?.value  ?? "";

  setStatus("saving…");
  try {
    await fetchJson(`/api/units/${encodeURIComponent(id)}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "application/json" },
      body: JSON.stringify({ content, type, tags, source }),
    });

    // Reload canonical detail from server
    const fresh = await fetchJson(`/api/units/${encodeURIComponent(id)}`);
    currentDetail = fresh;
    editMode = false;
    renderDetail(fresh);

    // Refresh card list in background so type/snippet changes are reflected
    refreshAllUnits();
    setStatus("saved.", "ok");
  } catch (e) {
    setStatus(`save failed: ${e.message}`, "error");
  }
}

async function doClone(id) {
  if (!id) return;
  setStatus("cloning…");
  try {
    const data  = await fetchJson(`/api/units/${encodeURIComponent(id)}/clone`, { method: "POST" });
    const newId = data.id ?? data.unit?.id;
    setStatus(`cloned → ${shortId(newId ?? "")}`, "ok");
    await refreshAllUnits();
    if (newId) selectCard(newId);
  } catch (e) {
    setStatus(`clone failed: ${e.message}`, "error");
  }
}

async function doArchive(id, currentlyArchived) {
  if (!id) return;
  const action = currentlyArchived ? "unarchive" : "archive";
  setStatus(`${action}ing…`);
  try {
    await fetchJson(`/api/units/${encodeURIComponent(id)}/${action}`, { method: "POST" });
    setStatus(`${action}d.`, "ok");
    await refreshAllUnits();
    // Re-load detail so archived badge and button label update
    const fresh = await fetchJson(`/api/units/${encodeURIComponent(id)}`);
    currentDetail = fresh;
    renderDetail(fresh);
  } catch (e) {
    setStatus(`${action} failed: ${e.message}`, "error");
  }
}

async function refreshAllUnits() {
  try {
    const data = await fetchJson("/api/search?q=");
    allUnits = data.units ?? [];
    if (!searchQuery) searchUnits = allUnits;
    renderChips();
    renderCards();
    elCount.textContent = `${allUnits.length} units`;
  } catch { /* silent refresh failure */ }
}

// ── Helpers ────────────────────────────────────────────────────────────────
function esc(s) {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function shortId(id) {
  return id ? id.split("-")[0] : "";
}

// Parse leading `# Heading` from unit content. Returns { headline, body }
// where body has the heading line stripped to avoid duplication in the
// rendered output (same logic as wiki.js).
function splitHeadline(content) {
  if (!content) return { headline: "", body: "" };
  const lines = content.split("\n");
  let i = 0;
  while (i < lines.length && !lines[i].trim()) i++;
  if (i >= lines.length) return { headline: "", body: content };
  const m = /^#\s+(.+?)\s*$/.exec(lines[i]);
  if (!m) return { headline: "", body: content };
  const rest = lines.slice(i + 1);
  if (rest[0]?.trim() === "") rest.shift();
  return { headline: m[1].trim(), body: rest.join("\n") };
}

// Render markdown via the marked CDN script (loaded in browse.html). Degrades
// to pre-formatted escaped text if CDN failed to load.
function renderMarkdown(md) {
  if (!md) return "<em style='color:var(--fg-muted)'>no content</em>";
  if (typeof window.marked === "undefined")
    return `<pre style="white-space:pre-wrap;font:12px var(--mono);color:var(--fg)">${esc(md)}</pre>`;
  return window.marked.parse(md, { gfm: true, breaks: false });
}

function debounce(fn, ms) {
  let timer;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

init();
