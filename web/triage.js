/**
 * triage.js — Layer 4 scan-first homepage.
 *
 * Three-pane layout: sidebar | list | detail.
 * Fetches /api/scan/counts + /api/scan/:cat on demand.
 * Actions: archive, verify (single = no confirm; bulk = confirm dialog).
 */

// ── State ──────────────────────────────────────────────────────────────────

const STALE_KEY = 'triage_stale_days';

let state = {
  staleDays: parseInt(localStorage.getItem(STALE_KEY) || '30', 10),
  activeCat: null,
  items: [],          // current list items (ScanItem[])
  contraItems: [],    // current contradiction pairs
  selectedIds: new Set(),
  openItemId: null,   // ID shown in detail pane
  counts: null,       // ScanCounts object
  scanTime: null,
};

// ── DOM refs ───────────────────────────────────────────────────────────────

const $ = id => document.getElementById(id);
const navBadge     = $('nav-badge');
const rescanBtn    = $('rescan-btn');
const scanTs       = $('scan-ts');
const staleSeg     = $('stale-seg');
const countEls     = {
  degraded:       $('cnt-degraded'),
  contradictions: $('cnt-contradictions'),
  oversized:      $('cnt-oversized'),
  orphaned:       $('cnt-orphaned'),
  stale:          $('cnt-stale'),
};
const catBtns      = document.querySelectorAll('.triage__cat-btn');
const listTitle    = $('list-title');
const listDesc     = $('list-desc');
const toolbarCount = $('toolbar-count');
const selectAllCb  = $('select-all-cb');
const btnVerifyAll = $('btn-verify-all');
const btnArchiveAll= $('btn-archive-all');
const itemList     = $('item-list');
const detailPane   = $('detail-pane');
const detailTitle  = $('detail-title');
const detailClose  = $('detail-close');
const detailBody   = $('detail-body');
const detailActions= $('detail-actions');
const healthBlock  = $('health-block');
const healthLabel  = $('health-label');
const healthDetail = $('health-detail');
const confirmDialog= $('confirm-dialog');
const confirmTitle = $('confirm-title');
const confirmDesc  = $('confirm-desc');
const confirmCancel= $('confirm-cancel');
const confirmOk    = $('confirm-ok');
const statusBar    = $('status');

// ── Category metadata ──────────────────────────────────────────────────────

const CAT_META = {
  degraded: {
    label: 'Degraded',
    desc: 'Units with wrong or outdated marks — human signal of stale truth.',
    dot: '#f87171',
    cntClass: 'triage__cnt-crit',
  },
  contradictions: {
    label: 'Contradictions',
    desc: 'Units linked by contradicts — two claims that cannot both be true.',
    dot: '#fb923c',
    cntClass: 'triage__cnt-warn',
  },
  oversized: {
    label: 'Oversized',
    desc: 'Units above the byte-size warn threshold (default 2 KB). Split them.',
    dot: '#eab308',
    cntClass: 'triage__cnt-caution',
  },
  orphaned: {
    label: 'Orphaned',
    desc: 'No links in or out, older than 14 days. Forgotten or mis-filed.',
    dot: 'var(--fg-dim)',
    cntClass: 'triage__cnt-muted',
  },
  stale: {
    label: 'Stale',
    desc: 'procedure / principle units with no marks, past the stale cutoff.',
    dot: 'var(--fg-muted)',
    cntClass: 'triage__cnt-muted',
  },
};

// Type accent colors (mirrors styles.css)
const TYPE_COLORS = {
  principle: '#818cf8',
  fact:      '#38bdf8',
  lesson:    '#fb923c',
  procedure: '#4ade80',
  aspect:    '#c084fc',
  idea:      '#facc15',
  preference:'#f472b6',
};

function typeColor(t) { return TYPE_COLORS[t] || 'var(--fg-muted)'; }

// ── Init ───────────────────────────────────────────────────────────────────

initStaleSeg();
const _hashCat = window.location.hash.replace('#', '');
loadCounts().then(() => {
  if (_hashCat && CAT_META[_hashCat]) selectCat(_hashCat);
});
catBtns.forEach(btn => btn.addEventListener('click', () => selectCat(btn.dataset.cat)));
rescanBtn.addEventListener('click', () => loadCounts(true));
selectAllCb.addEventListener('change', onSelectAll);
btnVerifyAll.addEventListener('click', onVerifyAll);
btnArchiveAll.addEventListener('click', onArchiveAll);
detailClose.addEventListener('click', closeDetail);
confirmCancel.addEventListener('click', () => confirmDialog.close('cancel'));

// ── Stale segment control ──────────────────────────────────────────────────

function initStaleSeg() {
  staleSeg.querySelectorAll('button').forEach(btn => {
    const days = parseInt(btn.dataset.days, 10);
    if (days === state.staleDays) btn.style.background = 'var(--accent-bg)', btn.style.color = 'var(--accent)';
    btn.addEventListener('click', () => {
      state.staleDays = days;
      localStorage.setItem(STALE_KEY, days);
      staleSeg.querySelectorAll('button').forEach(b => {
        const on = parseInt(b.dataset.days, 10) === days;
        b.style.background = on ? 'var(--accent-bg)' : '';
        b.style.color = on ? 'var(--accent)' : '';
      });
      loadCounts(true);
      if (state.activeCat === 'stale') loadCategory('stale');
    });
  });
}

// ── Data fetchers ──────────────────────────────────────────────────────────

async function loadCounts(silent = false) {
  if (!silent) setStatus('scanning…');
  try {
    const res = await fetch(`/api/scan/counts?stale_days=${state.staleDays}`);
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    state.counts = await res.json();
    state.scanTime = new Date();
    renderCounts();
    updateNavBadge();
    updateHealth();
    setStatus('scan complete.');
    updateScanTs();
  } catch (err) {
    setStatus(`scan error: ${err.message}`, true);
  }
}

async function loadCategory(cat) {
  setStatus(`loading ${cat}…`);
  const url = cat === 'stale'
    ? `/api/scan/${cat}?stale_days=${state.staleDays}`
    : `/api/scan/${cat}`;
  try {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    const data = await res.json();
    if (cat === 'contradictions') {
      state.contraItems = data;
      state.items = [];
    } else {
      state.items = data;
      state.contraItems = [];
    }
    state.selectedIds.clear();
    renderList(cat);
    setStatus(`${cat}: ${data.length} items.`);
  } catch (err) {
    setStatus(`error loading ${cat}: ${err.message}`, true);
  }
}

// ── Rendering ──────────────────────────────────────────────────────────────

function renderCounts() {
  const c = state.counts;
  if (!c) return;
  ['degraded','contradictions','oversized','orphaned','stale'].forEach(cat => {
    const el = countEls[cat];
    const n = c[cat];
    el.textContent = n;
    const meta = CAT_META[cat];
    el.className = `triage__cat-count ${n > 0 ? meta.cntClass : 'none'}`;
  });
}

function updateNavBadge() {
  const c = state.counts;
  if (!c) return;
  const critical = c.degraded + c.contradictions;
  navBadge.textContent = critical;
  navBadge.style.display = critical > 0 ? '' : 'none';
}

function updateHealth() {
  const c = state.counts;
  if (!c) return;
  const critical = c.degraded + c.contradictions;
  const hygiene = c.oversized + c.orphaned + c.stale;
  const total = critical + hygiene;
  if (total === 0) {
    healthBlock.className = 'triage__health clean';
    healthLabel.textContent = '✓ Store is clean';
    healthDetail.textContent = '0 items need attention';
  } else {
    healthBlock.className = 'triage__health dirty';
    healthLabel.textContent = `⚠ ${total} need attention`;
    const parts = [];
    if (c.degraded) parts.push(`${c.degraded} degraded`);
    if (c.contradictions) parts.push(`${c.contradictions} contradiction${c.contradictions !== 1 ? 's' : ''}`);
    if (c.oversized) parts.push(`${c.oversized} oversized`);
    if (c.orphaned) parts.push(`${c.orphaned} orphaned`);
    if (c.stale) parts.push(`${c.stale} stale (${state.staleDays}d)`);
    healthDetail.innerHTML = parts.join(' · ');
  }
}

function updateScanTs() {
  if (!state.scanTime) return;
  const sec = Math.round((Date.now() - state.scanTime.getTime()) / 1000);
  scanTs.textContent = sec < 5 ? 'just now' : `${sec}s ago`;
}

function selectCat(cat) {
  state.activeCat = cat;
  catBtns.forEach(b => b.classList.toggle('active', b.dataset.cat === cat));
  const meta = CAT_META[cat];
  listTitle.innerHTML = `<span style="display:inline-block;width:9px;height:9px;border-radius:50%;background:${meta.dot};margin-right:5px;flex-shrink:0;"></span>${meta.label}`;
  listDesc.textContent = meta.desc;
  closeDetail();
  loadCategory(cat);
}

function renderList(cat) {
  selectAllCb.checked = false;
  updateBatchButtons();

  if (cat === 'contradictions') {
    renderContraList();
    return;
  }

  const items = state.items;
  if (items.length === 0) {
    itemList.innerHTML = renderEmptyState(cat);
    toolbarCount.textContent = '0 items';
    return;
  }

  toolbarCount.textContent = `${items.length} item${items.length !== 1 ? 's' : ''}`;
  itemList.innerHTML = items.map((item, i) => renderRow(item, i, cat)).join('');

  // Wire row events
  itemList.querySelectorAll('.triage__row').forEach(row => {
    const id = row.dataset.id;
    row.querySelector('.triage__row-cb input').addEventListener('change', e => {
      e.stopPropagation();
      onRowCheck(id, e.target.checked);
    });
    row.querySelector('.triage__act-btn.archive').addEventListener('click', e => {
      e.stopPropagation();
      archiveItem(id, row);
    });
    row.querySelector('.triage__act-btn.verify').addEventListener('click', e => {
      e.stopPropagation();
      verifyItem(id, row);
    });
    row.querySelector('.triage__act-btn.open').addEventListener('click', e => {
      e.stopPropagation();
      openDetail(id);
    });
    row.addEventListener('click', () => openDetail(id));
  });
}

function renderRow(item, i, cat) {
  const color = typeColor(item.type);
  const headline = firstLine(item.snippet);
  const markHtml = item.latest_mark_kind
    ? `<span class="triage__mark-chip mark-${item.latest_mark_kind}">${item.latest_mark_kind}</span>`
    : '';
  const tagsHtml = (item.tags || []).slice(0, 3)
    .map(t => `<span class="triage__tag-chip">${escHtml(t)}</span>`).join('');
  const ageHtml = `<span class="triage__meta-age">${item.age_days}d</span>`;
  const bytesHtml = cat === 'oversized'
    ? `<span class="triage__meta-bytes">${fmtBytes(item.byte_size)}</span>`
    : '';
  const typeStyle = `background:${color}22;color:${color};border:1px solid ${color}44;`;

  return `<div class="triage__row" data-id="${escHtml(item.id)}">
    <div class="triage__row-cb"><input type="checkbox" data-id="${escHtml(item.id)}"></div>
    <div class="triage__type-pip" style="background:${color}"></div>
    <div class="triage__row-body">
      <div class="triage__row-headline">${escHtml(headline)}</div>
      <div class="triage__row-meta">
        <span class="triage__type-chip" style="${typeStyle}">${escHtml(item.type)}</span>
        ${markHtml}
        ${bytesHtml}
        ${tagsHtml}
        ${ageHtml}
      </div>
    </div>
    <div class="triage__row-actions">
      <button class="triage__act-btn archive" title="Archive">Archive</button>
      <button class="triage__act-btn verify" title="Mark verified">✓ Verify</button>
      <button class="triage__act-btn open" title="Open detail">Open →</button>
    </div>
  </div>`;
}

function renderContraList() {
  const items = state.contraItems;
  toolbarCount.textContent = `${items.length} pair${items.length !== 1 ? 's' : ''}`;
  // Batch ops less meaningful for contradiction pairs — hide checkboxes
  selectAllCb.style.display = 'none';

  if (items.length === 0) {
    itemList.innerHTML = renderEmptyState('contradictions');
    return;
  }

  itemList.innerHTML = items.map(pair => renderContraRow(pair)).join('');

  itemList.querySelectorAll('.triage__contra-row').forEach(row => {
    const fromId = row.dataset.from;
    const toId = row.dataset.to;
    row.querySelector('.btn-open-from').addEventListener('click', e => {
      e.stopPropagation();
      openDetail(fromId);
    });
    row.querySelector('.btn-open-to').addEventListener('click', e => {
      e.stopPropagation();
      openDetail(toId);
    });
  });
}

function renderContraRow(pair) {
  return `<div class="triage__contra-row" data-from="${escHtml(pair.from_id)}" data-to="${escHtml(pair.to_id)}">
    <div class="triage__contra-pair">
      <div class="triage__contra-side">
        <div class="triage__contra-id">${pair.from_id.substring(0,12)}…
          <span class="triage__type-chip" style="font:600 9px var(--mono);padding:0 4px;border-radius:3px;background:${typeColor(pair.from_type)}22;color:${typeColor(pair.from_type)};border:1px solid ${typeColor(pair.from_type)}44;">${escHtml(pair.from_type)}</span>
        </div>
        <div class="triage__contra-text">${escHtml(firstLine(pair.from_snippet))}</div>
      </div>
      <div class="triage__contra-vs">VS</div>
      <div class="triage__contra-side">
        <div class="triage__contra-id">${pair.to_id.substring(0,12)}…
          <span class="triage__type-chip" style="font:600 9px var(--mono);padding:0 4px;border-radius:3px;background:${typeColor(pair.to_type)}22;color:${typeColor(pair.to_type)};border:1px solid ${typeColor(pair.to_type)}44;">${escHtml(pair.to_type)}</span>
        </div>
        <div class="triage__contra-text">${escHtml(firstLine(pair.to_snippet))}</div>
      </div>
    </div>
    <div class="triage__contra-actions">
      <button class="triage__act-btn open btn-open-from">Open A →</button>
      <button class="triage__act-btn open btn-open-to">Open B →</button>
    </div>
  </div>`;
}

function renderEmptyState(cat) {
  const isClean = state.counts && state.counts[cat] === 0;
  return `<div class="triage__empty">
    <div class="triage__empty-icon">${isClean ? '✓' : '—'}</div>
    <div class="triage__empty-title">${isClean ? 'All clear' : 'Nothing here'}</div>
    <div class="triage__empty-sub">${isClean ? `No ${CAT_META[cat]?.label.toLowerCase() || cat} items found.` : 'No items loaded.'}</div>
  </div>`;
}

// ── Detail pane ────────────────────────────────────────────────────────────

async function openDetail(id) {
  // Find item in current items
  let item = state.items.find(i => i.id === id);
  let fromContra = null;
  if (!item) {
    // Might be a contradiction side — fetch directly
    fromContra = id;
  }

  if (!item && fromContra) {
    // Fetch unit from server
    try {
      const res = await fetch(`/api/units/${id}`);
      if (!res.ok) throw new Error(res.statusText);
      const unit = await res.json();
      item = unitToScanItem(unit);
    } catch (err) {
      setStatus(`failed to load unit: ${err.message}`, true);
      return;
    }
  }

  if (!item) return;

  state.openItemId = id;
  detailPane.classList.remove('hidden');

  // Mark row selected
  document.querySelectorAll('.triage__row').forEach(r => {
    r.classList.toggle('selected', r.dataset.id === id);
  });

  detailTitle.textContent = firstLine(item.snippet);

  const color = typeColor(item.type);
  const typeStyle = `background:${color}22;color:${color};border:1px solid ${color}44;display:inline-block;padding:1px 6px;border-radius:4px;font:600 10px var(--mono);text-transform:uppercase;`;
  const tagsHtml = (item.tags || []).map(t =>
    `<span style="padding:2px 7px;font:11px var(--mono);background:var(--bg-elev);border:1px solid var(--border);border-radius:99px;color:var(--fg-muted);">${escHtml(t)}</span>`
  ).join('');

  detailBody.innerHTML = `
    <div class="triage__detail-field">
      <div class="triage__detail-label">ID</div>
      <div class="triage__detail-value mono">${escHtml(item.id)}</div>
    </div>
    <div class="triage__detail-field">
      <div class="triage__detail-label">Type</div>
      <div class="triage__detail-value"><span style="${typeStyle}">${escHtml(item.type)}</span></div>
    </div>
    ${item.latest_mark_kind ? `
    <div class="triage__detail-field">
      <div class="triage__detail-label">Latest mark</div>
      <div class="triage__detail-value"><span class="triage__mark-chip mark-${escHtml(item.latest_mark_kind)}">${escHtml(item.latest_mark_kind)}</span></div>
    </div>` : ''}
    <div class="triage__detail-field">
      <div class="triage__detail-label">Age / Size</div>
      <div class="triage__detail-value">${item.age_days}d · ${fmtBytes(item.byte_size)}</div>
    </div>
    ${tagsHtml ? `
    <div class="triage__detail-field">
      <div class="triage__detail-label">Tags</div>
      <div class="triage__detail-tags">${tagsHtml}</div>
    </div>` : ''}
    <div class="triage__detail-field">
      <div class="triage__detail-label">Confidence</div>
      <div class="triage__detail-value">${item.confidence.toFixed(2)}</div>
    </div>
    <div class="triage__detail-field">
      <div class="triage__detail-label">Content</div>
      <div class="triage__detail-content">${escHtml(item.snippet)}</div>
    </div>
  `;

  detailActions.innerHTML = `
    <button class="triage__detail-act verify" id="detail-verify">✓ Verify</button>
    <button class="triage__detail-act archive" id="detail-archive">Archive</button>
    <button class="triage__detail-act" id="detail-browse">Browse →</button>
  `;

  document.getElementById('detail-verify').addEventListener('click', () => {
    verifyItem(id, null);
  });
  document.getElementById('detail-archive').addEventListener('click', () => {
    const row = document.querySelector(`.triage__row[data-id="${id}"]`);
    archiveItem(id, row);
    closeDetail();
  });
  document.getElementById('detail-browse').addEventListener('click', () => {
    window.location.href = `/browse#${id}`;
  });
}

function closeDetail() {
  state.openItemId = null;
  detailPane.classList.add('hidden');
  document.querySelectorAll('.triage__row').forEach(r => r.classList.remove('selected'));
}

// ── Actions ────────────────────────────────────────────────────────────────

async function archiveItem(id, rowEl) {
  if (rowEl) rowEl.classList.add('fading');
  try {
    const res = await fetch(`/api/units/${id}/archive`, { method: 'POST' });
    if (!res.ok) throw new Error(await res.text());
    // Remove from state
    state.items = state.items.filter(i => i.id !== id);
    state.selectedIds.delete(id);
    if (rowEl) rowEl.remove();
    updateToolbarCount();
    // Decrement sidebar count
    if (state.activeCat && state.counts) {
      state.counts[state.activeCat] = Math.max(0, (state.counts[state.activeCat] || 1) - 1);
      renderCounts();
      updateNavBadge();
      updateHealth();
    }
    setStatus('archived.');
  } catch (err) {
    if (rowEl) rowEl.classList.remove('fading');
    setStatus(`archive failed: ${err.message}`, true);
  }
}

async function verifyItem(id, rowEl) {
  try {
    const res = await fetch(`/api/units/${id}/verify`, { method: 'POST' });
    if (!res.ok) throw new Error(await res.text());
    setStatus('verified.');
    // Remove from degraded/stale lists since verifying is a mark that reduces stale count
    if (state.activeCat === 'degraded' || state.activeCat === 'stale') {
      state.items = state.items.filter(i => i.id !== id);
      if (rowEl) rowEl.remove();
      updateToolbarCount();
    }
  } catch (err) {
    setStatus(`verify failed: ${err.message}`, true);
  }
}

// ── Batch actions ──────────────────────────────────────────────────────────

function onRowCheck(id, checked) {
  if (checked) {
    state.selectedIds.add(id);
  } else {
    state.selectedIds.delete(id);
  }
  updateBatchButtons();
  // Sync select-all checkbox
  const totalRows = itemList.querySelectorAll('.triage__row').length;
  selectAllCb.checked = totalRows > 0 && state.selectedIds.size === totalRows;
}

function onSelectAll() {
  const checked = selectAllCb.checked;
  itemList.querySelectorAll('.triage__row').forEach(row => {
    const id = row.dataset.id;
    const cb = row.querySelector('input[type=checkbox]');
    if (cb) cb.checked = checked;
    if (checked) state.selectedIds.add(id);
    else state.selectedIds.delete(id);
  });
  updateBatchButtons();
}

function updateBatchButtons() {
  const n = state.selectedIds.size;
  btnVerifyAll.disabled = n === 0;
  btnArchiveAll.disabled = n === 0;
  btnVerifyAll.textContent = n > 0 ? `✓ Verify ${n}` : '✓ Verify all';
  btnArchiveAll.textContent = n > 0 ? `Archive ${n}` : 'Archive all';
}

function updateToolbarCount() {
  const n = state.items.length;
  toolbarCount.textContent = `${n} item${n !== 1 ? 's' : ''}`;
}

async function onVerifyAll() {
  const ids = [...state.selectedIds];
  if (ids.length === 0) return;
  const ok = await showConfirm(
    `Verify ${ids.length} item${ids.length !== 1 ? 's' : ''}?`,
    'This sets verified=true on each selected unit. No content is changed.'
  );
  if (!ok) return;
  setStatus(`verifying ${ids.length} items…`);
  let done = 0;
  for (const id of ids) {
    try {
      await fetch(`/api/units/${id}/verify`, { method: 'POST' });
      done++;
      const row = document.querySelector(`.triage__row[data-id="${id}"]`);
      if (row) row.remove();
      state.items = state.items.filter(i => i.id !== id);
      state.selectedIds.delete(id);
    } catch (_) {}
  }
  updateToolbarCount();
  selectAllCb.checked = false;
  updateBatchButtons();
  setStatus(`verified ${done} items.`);
}

async function onArchiveAll() {
  const ids = [...state.selectedIds];
  if (ids.length === 0) return;
  const ok = await showConfirm(
    `Archive ${ids.length} item${ids.length !== 1 ? 's' : ''}?`,
    'Units will be soft-deleted. This is reversible via simaris unarchive.'
  );
  if (!ok) return;
  setStatus(`archiving ${ids.length} items…`);
  let done = 0;
  for (const id of ids) {
    try {
      await fetch(`/api/units/${id}/archive`, { method: 'POST' });
      done++;
      const row = document.querySelector(`.triage__row[data-id="${id}"]`);
      if (row) row.classList.add('fading');
      state.items = state.items.filter(i => i.id !== id);
      state.selectedIds.delete(id);
    } catch (_) {}
  }
  // Remove faded rows after animation
  setTimeout(() => {
    document.querySelectorAll('.triage__row.fading').forEach(r => r.remove());
    updateToolbarCount();
  }, 350);
  if (state.activeCat && state.counts) {
    state.counts[state.activeCat] = Math.max(0, (state.counts[state.activeCat] || 0) - done);
    renderCounts();
    updateNavBadge();
    updateHealth();
  }
  selectAllCb.checked = false;
  updateBatchButtons();
  setStatus(`archived ${done} items.`);
}

// ── Confirm dialog ─────────────────────────────────────────────────────────

function showConfirm(title, desc) {
  confirmTitle.textContent = title;
  confirmDesc.textContent = desc;
  confirmDialog.showModal();
  return new Promise(resolve => {
    const onOk = () => { confirmDialog.close('ok'); resolve(true); };
    const onCancel = () => { confirmDialog.close('cancel'); resolve(false); };
    confirmOk.onclick = onOk;
    confirmCancel.onclick = onCancel;
    confirmDialog.onclose = () => resolve(confirmDialog.returnValue === 'ok');
  });
}

// ── Utility ────────────────────────────────────────────────────────────────

function firstLine(text) {
  if (!text) return '(empty)';
  return (text.split('\n')[0] || text).trim().substring(0, 120);
}

function escHtml(s) {
  if (!s) return '';
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function fmtBytes(n) {
  if (n === null || n === undefined) return '—';
  if (n < 1024) return `${n}B`;
  return `${(n / 1024).toFixed(1)}KB`;
}

function setStatus(msg, isError = false) {
  statusBar.textContent = msg;
  statusBar.className = 'status' + (isError ? ' status--error' : '');
}

function unitToScanItem(unit) {
  return {
    id: unit.id,
    type: unit.type || unit.unit_type,
    slug: null,
    byte_size: (unit.content || '').length,
    tags: unit.tags || [],
    confidence: unit.confidence || 1.0,
    archived: unit.archived || false,
    snippet: (unit.content || '').substring(0, 200),
    latest_mark_kind: null,
    age_days: 0,
  };
}

// Auto-refresh scan timestamp every 30s
setInterval(updateScanTs, 30_000);

// Hash deep-link is handled at init above.
