// Consolidation review pane.
//
// Loads /api/consolidation/clusters into a left list; selecting a cluster
// fetches each member's full content and renders the side-by-side review
// grid on the right; submit triggers /api/consolidation/action.
//
// Phase 1 — actions: archive, retype, skip. Merge UI is disabled (501).

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

const state = {
  report: null,
  filteredClusters: [],
  activePattern: 'all',
  activeClusterId: null,
  memberContent: new Map(), // id -> full content string
};

function setStatus(msg) {
  $('#status').textContent = msg;
}

function setSummary(report) {
  const s = report?.summary;
  if (!s) {
    $('#cons-summary').textContent = '';
    return;
  }
  const parts = [`total_units=${s.total_units}`, `clusters=${s.cluster_count}`];
  if (s.by_pattern && Object.keys(s.by_pattern).length) {
    const bp = Object.entries(s.by_pattern)
      .map(([k, v]) => `${k}=${v}`)
      .join(', ');
    parts.push(`[${bp}]`);
  }
  $('#cons-summary').textContent = parts.join('  ');
}

// ── Cluster list rendering ────────────────────────────────────────────────

let patternFilterListenerBound = false;
function buildPatternFilters(report) {
  const container = $('#cons-pattern-filters');
  // Reset, leaving only the 'all' chip.
  container.innerHTML = '';
  const allBtn = chip('all', 'all', state.activePattern === 'all');
  container.appendChild(allBtn);
  const pats = Object.keys(report.summary?.by_pattern || {}).sort();
  for (const pat of pats) {
    const count = report.summary.by_pattern[pat];
    const btn = chip(pat, `${pat} (${count})`, state.activePattern === pat);
    container.appendChild(btn);
  }
  if (!patternFilterListenerBound) {
    container.addEventListener('click', (ev) => {
      const target = ev.target.closest('.cons__pat-chip');
      if (!target) return;
      state.activePattern = target.dataset.pattern;
      $$('.cons__pat-chip').forEach((c) =>
        c.classList.toggle('active', c.dataset.pattern === state.activePattern),
      );
      renderClusterList();
    });
    patternFilterListenerBound = true;
  }
}

function chip(pattern, label, active) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'cons__pat-chip' + (active ? ' active' : '');
  b.dataset.pattern = pattern;
  b.textContent = label;
  return b;
}

function renderClusterList() {
  if (!state.report) return;
  const all = state.report.clusters || [];
  state.filteredClusters =
    state.activePattern === 'all'
      ? all
      : all.filter((c) => (c.patterns || []).includes(state.activePattern));
  const list = $('#cons-cluster-list');
  list.innerHTML = '';
  if (state.filteredClusters.length === 0) {
    list.innerHTML = '<div style="padding: 20px; color: var(--fg-muted); font: 12px system-ui">no clusters match this filter.</div>';
    return;
  }
  for (const c of state.filteredClusters) {
    const card = document.createElement('div');
    card.className = 'cons__cluster-card' + (state.activeClusterId === c.cluster_id ? ' active' : '');
    card.dataset.clusterId = c.cluster_id;

    const line1 = document.createElement('div');
    line1.className = 'cons__cluster-line1';
    const idEl = document.createElement('span');
    idEl.className = 'cons__cluster-id';
    idEl.textContent = c.cluster_id;
    const sizeEl = document.createElement('span');
    sizeEl.className = 'cons__cluster-size';
    const avg = typeof c.avg_vec_sim === 'number' ? c.avg_vec_sim.toFixed(2) : '—';
    sizeEl.textContent = `${(c.members || []).length} members · vec_sim=${avg}`;
    line1.appendChild(idEl);
    line1.appendChild(sizeEl);

    const patTags = document.createElement('div');
    patTags.className = 'cons__pat-tags';
    for (const p of c.patterns || []) {
      const t = document.createElement('span');
      t.className = `cons__pat-tag cons__pat-tag--${p}`;
      t.textContent = p;
      patTags.appendChild(t);
    }

    const suggested = document.createElement('div');
    suggested.className = 'cons__cluster-suggested';
    suggested.textContent = c.suggested_action || '';

    card.appendChild(line1);
    card.appendChild(patTags);
    card.appendChild(suggested);
    card.addEventListener('click', () => selectCluster(c.cluster_id));
    list.appendChild(card);
  }
}

// ── Detail pane ───────────────────────────────────────────────────────────

async function selectCluster(clusterId) {
  state.activeClusterId = clusterId;
  $$('.cons__cluster-card').forEach((card) => {
    card.classList.toggle('active', card.dataset.clusterId === clusterId);
  });
  const cluster = state.filteredClusters.find((c) => c.cluster_id === clusterId);
  if (!cluster) return;
  await renderDetail(cluster);
}

async function renderDetail(cluster) {
  const detail = $('#cons-detail');
  detail.innerHTML = '';

  // Header
  const header = document.createElement('div');
  header.className = 'cons__detail-header';
  const title = document.createElement('div');
  title.className = 'cons__detail-title';
  title.textContent = `Cluster ${cluster.cluster_id}`;
  const meta = document.createElement('div');
  meta.className = 'cons__detail-meta';
  const avg = typeof cluster.avg_vec_sim === 'number' ? cluster.avg_vec_sim.toFixed(3) : '—';
  meta.textContent = `${(cluster.members || []).length} members · avg_vec_sim=${avg} · patterns=[${(cluster.patterns || []).join(', ')}]`;
  header.appendChild(title);
  header.appendChild(meta);
  detail.appendChild(header);

  const reason = document.createElement('div');
  reason.className = 'cons__detail-meta';
  reason.style.marginBottom = '8px';
  reason.textContent = `reason: ${cluster.reason || ''} · suggested: ${cluster.suggested_action || ''}`;
  detail.appendChild(reason);

  // Action bar
  const actionBar = buildActionBar(cluster);
  detail.appendChild(actionBar);

  // Member grid
  const grid = document.createElement('div');
  grid.className = 'cons__members';
  detail.appendChild(grid);

  // Lazy-fetch member content; render placeholders first then fill in.
  for (const m of cluster.members || []) {
    const card = buildMemberCard(m);
    grid.appendChild(card);
  }
  await fetchAndRenderMemberContents(cluster.members || []);

  // Result box (kept hidden until first action returns)
  const result = document.createElement('div');
  result.id = 'cons-result-box';
  result.style.display = 'none';
  result.className = 'cons__result-box';
  detail.appendChild(result);
}

function buildActionBar(cluster) {
  const bar = document.createElement('div');
  bar.className = 'cons__action-bar';

  const actionSel = document.createElement('select');
  actionSel.id = 'cons-action-select';
  for (const [val, label] of [
    ['archive', 'archive non-canonical → supersedes canonical'],
    ['retype', 'retype all members'],
    ['skip', 'skip (hide cluster)'],
  ]) {
    const opt = document.createElement('option');
    opt.value = val;
    opt.textContent = label;
    actionSel.appendChild(opt);
  }
  // Pre-select based on suggested_action.
  const sugg = (cluster.suggested_action || '').toLowerCase();
  if (sugg.includes('archive')) actionSel.value = 'archive';
  else if (sugg.includes('retype')) actionSel.value = 'retype';
  // else default 'archive'.

  const typeSel = document.createElement('select');
  typeSel.id = 'cons-new-type';
  typeSel.style.display = 'none';
  for (const t of ['fact', 'principle', 'procedure', 'lesson', 'idea', 'preference', 'aspect']) {
    const opt = document.createElement('option');
    opt.value = t;
    opt.textContent = t;
    typeSel.appendChild(opt);
  }

  actionSel.addEventListener('change', () => {
    typeSel.style.display = actionSel.value === 'retype' ? '' : 'none';
  });

  const submit = document.createElement('button');
  submit.type = 'button';
  submit.className = 'cons__action-submit';
  submit.textContent = 'Submit';
  submit.addEventListener('click', () => submitAction(cluster, actionSel.value, typeSel.value));

  bar.appendChild(label('action:'));
  bar.appendChild(actionSel);
  bar.appendChild(label('type:'));
  bar.appendChild(typeSel);
  bar.appendChild(submit);
  return bar;
}

function label(text) {
  const el = document.createElement('span');
  el.style.font = '11px var(--mono)';
  el.style.color = 'var(--fg-muted)';
  el.textContent = text;
  return el;
}

function buildMemberCard(m) {
  const card = document.createElement('div');
  card.className = 'cons__member';
  card.dataset.id = m.id;

  const head = document.createElement('div');
  head.className = 'cons__member-head';
  const idEl = document.createElement('span');
  idEl.className = 'cons__member-id';
  idEl.textContent = m.slug ? `${m.slug}` : m.id.slice(0, 12);
  const metaEl = document.createElement('div');
  metaEl.className = 'cons__member-meta';
  const bits = [];
  bits.push(`type=${m.type}`);
  bits.push(`age=${m.age_days}d`);
  bits.push(`marks=${m.marks}`);
  bits.push(m.id.slice(0, 8));
  for (const b of bits) {
    const s = document.createElement('span');
    s.textContent = b;
    metaEl.appendChild(s);
  }
  head.appendChild(idEl);
  head.appendChild(metaEl);

  const content = document.createElement('pre');
  content.className = 'cons__member-content';
  content.textContent = m.content_preview || '(loading…)';

  // Controls: canonical radio + archive checkbox
  const controls = document.createElement('div');
  controls.className = 'cons__member-controls';
  const canonLabel = document.createElement('label');
  const canonInput = document.createElement('input');
  canonInput.type = 'radio';
  canonInput.name = 'cons-canonical';
  canonInput.value = m.id;
  canonInput.className = 'cons-canonical-input';
  canonLabel.appendChild(canonInput);
  canonLabel.appendChild(document.createTextNode('canonical'));

  const includeLabel = document.createElement('label');
  const includeInput = document.createElement('input');
  includeInput.type = 'checkbox';
  includeInput.checked = true;
  includeInput.value = m.id;
  includeInput.className = 'cons-member-include';
  includeLabel.appendChild(includeInput);
  includeLabel.appendChild(document.createTextNode('include'));

  controls.appendChild(canonLabel);
  controls.appendChild(includeLabel);

  card.appendChild(head);
  card.appendChild(content);
  card.appendChild(controls);
  return card;
}

async function fetchAndRenderMemberContents(members) {
  await Promise.all(
    members.map(async (m) => {
      if (state.memberContent.has(m.id)) {
        injectMemberContent(m.id, state.memberContent.get(m.id));
        return;
      }
      try {
        const r = await fetch(`/api/units/${encodeURIComponent(m.id)}`);
        if (!r.ok) throw new Error(`unit fetch ${r.status}`);
        const u = await r.json();
        const content = u.content || u.preview || m.content_preview || '';
        state.memberContent.set(m.id, content);
        injectMemberContent(m.id, content);
      } catch (e) {
        injectMemberContent(m.id, `(failed to load: ${e})`);
      }
    }),
  );
}

function injectMemberContent(id, content) {
  const card = document.querySelector(`.cons__member[data-id="${cssEscape(id)}"]`);
  if (!card) return;
  const pre = card.querySelector('.cons__member-content');
  if (pre) pre.textContent = content;
}

function cssEscape(s) {
  return s.replace(/"/g, '\\"');
}

// ── Action submit ─────────────────────────────────────────────────────────

async function submitAction(cluster, action, newType) {
  const memberIds = Array.from(document.querySelectorAll('.cons-member-include'))
    .filter((cb) => cb.checked)
    .map((cb) => cb.value);
  const canonicalEl = document.querySelector('.cons-canonical-input:checked');
  const canonicalId = canonicalEl ? canonicalEl.value : null;

  if (action === 'archive' && !canonicalId) {
    return showResult(false, 'pick a canonical first (radio in one member card).');
  }
  if (action === 'archive' && memberIds.length < 2) {
    return showResult(false, 'archive needs ≥ 2 included members (canonical + at least one to archive).');
  }
  if (action === 'retype' && memberIds.length === 0) {
    return showResult(false, 'retype needs ≥ 1 included member.');
  }

  const body = {
    cluster_id: cluster.cluster_id,
    action,
    canonical_id: canonicalId,
    member_ids: memberIds,
    new_type: action === 'retype' ? newType : null,
  };

  setStatus(`submitting ${action} for ${cluster.cluster_id}…`);
  try {
    const r = await fetch('/api/consolidation/action', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) {
      showResult(false, JSON.stringify(data, null, 2));
      setStatus(`action failed (${r.status})`);
      return;
    }
    showResult(data.ok !== false, JSON.stringify(data, null, 2));
    setStatus(`action ${action} ok — reloading clusters`);
    // Reload clusters (force refresh) so post-mutation state is reflected.
    await loadClusters({ refresh: true });
  } catch (e) {
    showResult(false, String(e));
    setStatus('action failed');
  }
}

function showResult(ok, text) {
  const box = $('#cons-result-box');
  if (!box) return;
  box.style.display = '';
  box.className = 'cons__result-box ' + (ok ? 'ok' : 'error');
  box.textContent = text;
}

// ── Top-level loader ──────────────────────────────────────────────────────

async function loadClusters({ refresh = false } = {}) {
  setStatus(refresh ? 'refreshing clusters…' : 'loading clusters…');
  const url = '/api/consolidation/clusters' + (refresh ? '?refresh=true' : '');
  try {
    const r = await fetch(url);
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const report = await r.json();
    state.report = report;
    state.activeClusterId = null;
    setSummary(report);
    buildPatternFilters(report);
    renderClusterList();
    $('#cons-detail').innerHTML =
      '<div class="cons__right-empty">Select a cluster on the left to review its members.</div>';
    setStatus(`clusters loaded — ${report.summary?.cluster_count || 0} surfaced`);
  } catch (e) {
    setStatus(`load failed: ${e}`);
  }
}

// ── Bootstrap ─────────────────────────────────────────────────────────────

$('#cons-refresh').addEventListener('click', () => loadClusters({ refresh: true }));
loadClusters({ refresh: false });
