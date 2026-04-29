// Units view — search, results table, modal with view/edit/clone/archive.
//
// All data flows through /api/* endpoints (server shells out to the
// simaris CLI). The seven legal knowledge types are loaded from
// /api/stats so the type dropdown stays in lockstep with the schema.

import { fetchJson, setStatus, shortTime } from "./app.js";

const FALLBACK_TYPES = [
  "fact",
  "procedure",
  "principle",
  "preference",
  "lesson",
  "idea",
  "aspect",
];

const els = {
  form: document.getElementById("search-form"),
  q: document.getElementById("search-q"),
  type: document.getElementById("search-type"),
  includeArchived: document.getElementById("search-include-archived"),
  meta: document.getElementById("search-meta"),
  body: document.getElementById("results-body"),
  empty: document.getElementById("results-empty"),
  modal: document.getElementById("unit-modal"),
  modalId: document.getElementById("modal-id"),
  modalView: document.getElementById("modal-view"),
  modalEdit: document.getElementById("modal-edit"),
  modalClose: document.getElementById("modal-close"),
  modalType: document.getElementById("modal-type"),
  modalTags: document.getElementById("modal-tags"),
  modalSource: document.getElementById("modal-source"),
  modalConfidence: document.getElementById("modal-confidence"),
  modalArchived: document.getElementById("modal-archived"),
  modalUpdated: document.getElementById("modal-updated"),
  modalContent: document.getElementById("modal-content"),
  modalLinks: document.getElementById("modal-links"),
  actionEdit: document.getElementById("action-edit"),
  actionClone: document.getElementById("action-clone"),
  actionArchive: document.getElementById("action-archive"),
  editForm: document.getElementById("edit-form"),
  editType: document.getElementById("edit-type"),
  editCancel: document.getElementById("edit-cancel"),
  toast: document.getElementById("toast"),
};

let validTypes = FALLBACK_TYPES;
let currentUnit = null; // last response from /api/units/:id

function fmtTags(tags) {
  if (!Array.isArray(tags) || tags.length === 0) return "";
  return tags.join(", ");
}

function fmtConfidence(c) {
  if (typeof c !== "number") return "—";
  return c.toFixed(2);
}

function escapeText(s) {
  return s == null ? "" : String(s);
}

function shortId(id) {
  if (!id) return "";
  // First chunk of UUIDv7 — enough to disambiguate while staying compact.
  return id.split("-")[0];
}

async function loadTypes() {
  try {
    const stats = await fetchJson("/api/stats");
    const keys = stats && stats.by_type ? Object.keys(stats.by_type) : [];
    if (keys.length > 0) {
      // Keep canonical order (FALLBACK_TYPES) where possible, then any extras.
      const ordered = [
        ...FALLBACK_TYPES.filter((t) => keys.includes(t)),
        ...keys.filter((t) => !FALLBACK_TYPES.includes(t)),
      ];
      validTypes = ordered;
    }
  } catch {
    // /api/stats unavailable — fall back to the static list.
  }
  renderTypeSelects();
}

function renderTypeSelects() {
  // Search filter (with "all types" sentinel).
  els.type.innerHTML =
    `<option value="">all types</option>` +
    validTypes.map((t) => `<option value="${t}">${t}</option>`).join("");
  // Edit form (no sentinel — every unit has a type).
  els.editType.innerHTML = validTypes
    .map((t) => `<option value="${t}">${t}</option>`)
    .join("");
}

function renderResults(payload) {
  els.body.innerHTML = "";
  const units = (payload && payload.units) || [];
  if (units.length === 0) {
    els.empty.hidden = false;
  } else {
    els.empty.hidden = true;
  }

  for (const u of units) {
    const tr = document.createElement("tr");
    tr.dataset.id = u.id;
    tr.innerHTML = `
      <td class="results__col-id"><code>${escapeHtml(shortId(u.id))}</code></td>
      <td class="results__col-type"><span class="badge badge--${escapeHtml(u.type || "")}">${escapeHtml(u.type || "")}</span></td>
      <td class="results__col-tags">${escapeHtml(fmtTags(u.tags))}</td>
      <td class="results__col-conf">${escapeHtml(fmtConfidence(u.confidence))}</td>
      <td class="results__col-snippet">${escapeHtml(u.snippet || "")}</td>
      <td class="results__col-actions">
        <button class="btn btn--small" data-act="view">View</button>
      </td>
    `;
    els.body.appendChild(tr);
  }

  els.meta.textContent = `${units.length} result${units.length === 1 ? "" : "s"} (${payload.kind})`;
}

function escapeHtml(s) {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

async function runSearch() {
  const q = els.q.value.trim();
  const type = els.type.value;
  const includeArchived = els.includeArchived.checked ? "1" : "";
  const params = new URLSearchParams();
  if (q) params.set("q", q);
  if (type) params.set("type", type);
  if (includeArchived) params.set("include_archived", includeArchived);
  const path = `/api/search?${params.toString()}`;
  setStatus(`searching: ${q || "(all)"} …`);
  try {
    const payload = await fetchJson(path);
    window.__simarisSearch = payload;
    renderResults(payload);
    setStatus(`${path} ok — ${shortTime()}`, "ok");
  } catch (err) {
    setStatus(`${path} failed: ${err.message}`, "error");
    els.body.innerHTML = "";
    els.empty.hidden = false;
  }
}

async function openModal(id) {
  setStatus(`loading unit ${shortId(id)} …`);
  try {
    const payload = await fetchJson(`/api/units/${encodeURIComponent(id)}`);
    currentUnit = payload;
    renderUnit(payload);
    showModalView();
    if (typeof els.modal.showModal === "function") {
      els.modal.showModal();
    } else {
      // Fallback for browsers without <dialog>.
      els.modal.setAttribute("open", "");
    }
    setStatus(`unit ${shortId(id)} ok — ${shortTime()}`, "ok");
  } catch (err) {
    setStatus(`unit ${shortId(id)} failed: ${err.message}`, "error");
  }
}

function renderUnit(payload) {
  const u = payload.unit || {};
  const links = payload.links || { incoming: [], outgoing: [] };
  els.modalId.textContent = u.id || "(no id)";
  els.modalType.textContent = u.type || "—";
  els.modalTags.textContent = fmtTags(u.tags) || "—";
  els.modalSource.textContent = u.source || "—";
  els.modalConfidence.textContent = fmtConfidence(u.confidence);
  els.modalArchived.textContent = u.archived ? "yes" : "no";
  els.modalUpdated.textContent = u.updated || "—";
  els.modalContent.textContent = u.content || "";
  els.modalLinks.innerHTML = "";

  const linkItems = [
    ...(links.outgoing || []).map((l) => ({ ...l, dir: "→" })),
    ...(links.incoming || []).map((l) => ({ ...l, dir: "←" })),
  ];
  if (linkItems.length === 0) {
    const li = document.createElement("li");
    li.className = "modal__link-empty";
    li.textContent = "no links";
    els.modalLinks.appendChild(li);
  } else {
    for (const l of linkItems) {
      const li = document.createElement("li");
      const other = l.dir === "→" ? l.to_id : l.from_id;
      li.innerHTML = `<span class="modal__link-rel">${escapeHtml(l.relationship)}</span> ${escapeHtml(l.dir)} <code>${escapeHtml(shortId(other))}</code>`;
      els.modalLinks.appendChild(li);
    }
  }

  // Sync the archive button label with current state.
  els.actionArchive.textContent = u.archived ? "Unarchive" : "Archive";
}

function closeModal() {
  if (typeof els.modal.close === "function") {
    els.modal.close();
  } else {
    els.modal.removeAttribute("open");
  }
  currentUnit = null;
  showModalView();
}

function showModalView() {
  els.modalView.hidden = false;
  els.modalEdit.hidden = true;
}

function showModalEdit() {
  if (!currentUnit) return;
  const u = currentUnit.unit || {};
  els.editForm.elements.content.value = u.content || "";
  // Type dropdown — default to current type if it matches a valid option.
  if (validTypes.includes(u.type)) {
    els.editForm.elements.type.value = u.type;
  }
  els.editForm.elements.tags.value = fmtTags(u.tags);
  els.editForm.elements.source.value = u.source || "";
  els.modalView.hidden = true;
  els.modalEdit.hidden = false;
}

function showToast(msg, level = "info") {
  els.toast.textContent = msg;
  els.toast.classList.remove("toast--ok", "toast--error");
  if (level === "ok") els.toast.classList.add("toast--ok");
  if (level === "error") els.toast.classList.add("toast--error");
  els.toast.hidden = false;
  clearTimeout(showToast._t);
  showToast._t = setTimeout(() => {
    els.toast.hidden = true;
  }, 3500);
}

async function saveEdit(e) {
  e.preventDefault();
  if (!currentUnit) return;
  const id = currentUnit.unit.id;
  const orig = currentUnit.unit;
  const fd = new FormData(els.editForm);
  const body = {};
  const content = String(fd.get("content") ?? "");
  const type = String(fd.get("type") ?? "");
  const tags = String(fd.get("tags") ?? "");
  const source = String(fd.get("source") ?? "");
  if (content !== (orig.content ?? "")) body.content = content;
  if (type && type !== (orig.type ?? "")) body.type = type;
  // Tags: send only when the comma-joined form differs.
  if (tags !== fmtTags(orig.tags)) body.tags = tags;
  if (source !== (orig.source ?? "")) body.source = source;
  if (Object.keys(body).length === 0) {
    showToast("no changes");
    return;
  }
  setStatus(`saving ${shortId(id)} …`);
  try {
    const res = await fetch(`/api/units/${encodeURIComponent(id)}`, {
      method: "POST",
      headers: { "content-type": "application/json", Accept: "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`${res.status} ${res.statusText}: ${text}`);
    }
    const updated = await res.json();
    currentUnit = updated;
    renderUnit(updated);
    showModalView();
    showToast("saved", "ok");
    setStatus(`saved ${shortId(id)} — ${shortTime()}`, "ok");
    runSearch();
  } catch (err) {
    setStatus(`save failed: ${err.message}`, "error");
    showToast(`save failed: ${err.message}`, "error");
  }
}

async function cloneCurrent() {
  if (!currentUnit) return;
  const id = currentUnit.unit.id;
  setStatus(`cloning ${shortId(id)} …`);
  try {
    const res = await fetch(`/api/units/${encodeURIComponent(id)}/clone`, {
      method: "POST",
      headers: { Accept: "application/json" },
    });
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    const data = await res.json();
    showToast(`cloned → ${shortId(data.id)}`, "ok");
    setStatus(`clone ok — ${shortId(data.id)} — ${shortTime()}`, "ok");
    runSearch();
    // Open the new clone for inspection.
    if (data.id) openModal(data.id);
  } catch (err) {
    setStatus(`clone failed: ${err.message}`, "error");
    showToast(`clone failed: ${err.message}`, "error");
  }
}

async function toggleArchive() {
  if (!currentUnit) return;
  const id = currentUnit.unit.id;
  const isArchived = !!currentUnit.unit.archived;
  const verb = isArchived ? "unarchive" : "archive";
  setStatus(`${verb} ${shortId(id)} …`);
  try {
    const res = await fetch(`/api/units/${encodeURIComponent(id)}/${verb}`, {
      method: "POST",
      headers: { Accept: "application/json" },
    });
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    showToast(`${verb}d`, "ok");
    setStatus(`${verb} ok — ${shortTime()}`, "ok");
    // Refresh modal + table state.
    const fresh = await fetchJson(`/api/units/${encodeURIComponent(id)}`);
    currentUnit = fresh;
    renderUnit(fresh);
    runSearch();
  } catch (err) {
    setStatus(`${verb} failed: ${err.message}`, "error");
    showToast(`${verb} failed: ${err.message}`, "error");
  }
}

// Wire up events.
els.form.addEventListener("submit", (e) => {
  e.preventDefault();
  runSearch();
});
els.includeArchived.addEventListener("change", runSearch);
els.type.addEventListener("change", runSearch);

els.body.addEventListener("click", (e) => {
  const btn = e.target.closest("button[data-act]");
  if (!btn) return;
  const tr = btn.closest("tr[data-id]");
  if (!tr) return;
  const id = tr.dataset.id;
  if (btn.dataset.act === "view") openModal(id);
});

els.modalClose.addEventListener("click", closeModal);
els.modal.addEventListener("close", () => {
  // ESC-to-close path. Reset the panel back to view mode so the next
  // open shows the meta/content section, not a stale edit form. Do NOT
  // null `currentUnit` here — the close event can fire after a quick
  // close/re-open cycle and would otherwise stomp the freshly-loaded
  // unit before the user clicks Edit.
  showModalView();
});
els.actionEdit.addEventListener("click", showModalEdit);
els.actionClone.addEventListener("click", cloneCurrent);
els.actionArchive.addEventListener("click", toggleArchive);
els.editForm.addEventListener("submit", saveEdit);
els.editCancel.addEventListener("click", showModalView);

// Boot.
loadTypes().then(() => {
  runSearch();
});
