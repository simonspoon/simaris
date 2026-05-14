// Dashboard view.
//
// Fetches /api/stats and renders four header tiles plus three ECharts
// charts (units-by-type donut, top-tags horizontal bar, confidence
// histogram). Auto-refreshes every 60s; the "Hide superseded" toggle
// re-renders client-side by subtracting `superseded_count` from the
// total tile (server-side filter lands when the CLI grows the flag —
// see task `zkrs`).

import { fetchJson, setStatus, shortTime } from "./app.js";

const REFRESH_MS = 60_000;
const TOP_TAGS = 20;

const TYPE_COLORS = {
  fact: "#2b6cb0",
  procedure: "#38a169",
  principle: "#805ad5",
  preference: "#d69e2e",
  lesson: "#dd6b20",
  idea: "#3182ce",
  aspect: "#e53e3e",
};

const charts = {
  types: null,
  tags: null,
  confidence: null,
};

let latestStats = null;
let refreshTimer = null;

function ensureChart(key, elId) {
  if (charts[key]) return charts[key];
  const el = document.getElementById(elId);
  if (!el || typeof window.echarts === "undefined") return null;
  charts[key] = window.echarts.init(el, "dark", { renderer: "canvas" });
  return charts[key];
}

// Common option overrides applied to every chart so the built-in echarts
// "dark" theme blends into the page (transparent bg, our card already
// supplies the surface) and tooltips/labels read on the dark backdrop.
const CHART_BASE = {
  backgroundColor: "transparent",
  textStyle: { color: "#d6d6d6" },
};

function fmtInt(n) {
  if (typeof n !== "number") return "—";
  return n.toLocaleString();
}

function renderTiles(stats, hideSuperseded) {
  const total = stats.total ?? 0;
  const superseded = stats.superseded_count ?? 0;
  const adjustedTotal = hideSuperseded ? Math.max(0, total - superseded) : total;

  document.getElementById("tile-total").textContent = fmtInt(adjustedTotal);
  const sub = document.getElementById("tile-total-sub");
  sub.textContent = hideSuperseded
    ? `excludes ${fmtInt(superseded)} superseded`
    : "";

  document.getElementById("tile-archived").textContent = fmtInt(
    stats.archived_count ?? 0,
  );
  document.getElementById("tile-superseded").textContent = fmtInt(superseded);
  document.getElementById("tile-inbox").textContent = fmtInt(
    stats.inbox_size ?? 0,
  );
}

function renderTypes(stats) {
  const chart = ensureChart("types", "chart-types");
  if (!chart) return;
  const byType = stats.by_type || {};
  const total = Object.values(byType).reduce((a, b) => a + b, 0);
  const data = Object.entries(byType)
    .map(([name, value]) => ({
      name,
      value,
      itemStyle: { color: TYPE_COLORS[name] || "#888" },
    }))
    .sort((a, b) => b.value - a.value);

  chart.setOption(
    {
      ...CHART_BASE,
      tooltip: { trigger: "item", formatter: "{b}: {c} ({d}%)" },
      legend: {
        bottom: 0,
        type: "scroll",
        textStyle: { color: "#d6d6d6" },
        formatter: (name) => {
          const item = data.find((d) => d.name === name);
          return `${name}  ${item ? item.value.toLocaleString() : 0}`;
        },
      },
      series: [
        {
          name: "type",
          type: "pie",
          radius: ["40%", "68%"],
          avoidLabelOverlap: false,
          label: {
            show: true,
            position: "center",
            formatter: () => `{total|${fmtInt(total)}}\n{sub|units}`,
            rich: {
              total: { fontSize: 22, fontWeight: "bold", color: "#e6e6e6", lineHeight: 30 },
              sub: { fontSize: 12, color: "#9a9a9a" },
            },
          },
          emphasis: {
            label: {
              show: true,
              formatter: (params) =>
                `{total|${params.value.toLocaleString()}}\n{sub|${params.name}}`,
            },
          },
          labelLine: { show: false },
          data,
        },
      ],
    },
    { notMerge: true },
  );
}

function renderTags(stats) {
  const chart = ensureChart("tags", "chart-tags");
  if (!chart) return;
  const top = ((stats.by_tag && stats.by_tag.top) || []).slice(0, TOP_TAGS);
  // Reverse so the largest bar is at the top of a horizontal bar chart.
  const ordered = [...top].reverse();
  const totalUnique = (stats.by_tag && stats.by_tag.total_unique) ?? top.length;

  chart.setOption(
    {
      ...CHART_BASE,
      tooltip: { trigger: "axis", axisPointer: { type: "shadow" } },
      grid: { left: 100, right: 24, top: 24, bottom: 36, containLabel: true },
      xAxis: { type: "value", name: "units" },
      yAxis: {
        type: "category",
        data: ordered.map((t) => t.tag),
        axisLabel: { fontFamily: "ui-monospace, SF Mono, Menlo, monospace" },
      },
      series: [
        {
          type: "bar",
          data: ordered.map((t) => t.count),
          itemStyle: { color: "#5b9bd5" },
          label: { show: true, position: "right", formatter: "{c}" },
        },
      ],
      graphic: [
        {
          type: "text",
          right: 8,
          bottom: 4,
          style: {
            text: `top ${top.length} of ${totalUnique} tags`,
            fill: "#9a9a9a",
            fontSize: 11,
          },
        },
      ],
    },
    { notMerge: true },
  );
}

function renderConfidence(stats) {
  const chart = ensureChart("confidence", "chart-confidence");
  if (!chart) return;
  const conf = stats.confidence || {};

  // "verified" (confidence ≥0.95) dominates at >99% of units — it swamps
  // the chart scale and makes the actionable buckets invisible. Split it
  // off as a callout; show only low/medium/high at their natural scale.
  const verifiedCount = conf.verified ?? 0;
  const buckets = [
    { name: "low (<0.6)", value: conf.low ?? 0, color: "#ef5350" },
    { name: "medium\n(0.6–0.8)", value: conf.medium ?? 0, color: "#e0b341" },
    { name: "high\n(0.8–0.95)", value: conf.high ?? 0, color: "#5b9bd5" },
  ];

  chart.setOption(
    {
      ...CHART_BASE,
      tooltip: { trigger: "axis", axisPointer: { type: "shadow" } },
      grid: { left: 24, right: 190, top: 24, bottom: 36, containLabel: true },
      xAxis: {
        type: "category",
        data: buckets.map((b) => b.name),
        axisLabel: { interval: 0 },
      },
      yAxis: { type: "value", name: "units" },
      graphic: [
        // Verified callout box — anchored to the right margin reserved by grid.right
        {
          type: "rect",
          right: 4,
          top: "center",
          shape: { width: 175, height: 96 },
          style: { fill: "rgba(102,187,106,0.06)", stroke: "#3a6b3e", lineWidth: 1 },
        },
        {
          type: "text",
          right: 12,
          top: "30%",
          style: {
            text: "VERIFIED (≥0.95)",
            fill: "#66bb6a",
            font: "600 10px system-ui, sans-serif",
          },
        },
        {
          type: "text",
          right: 12,
          top: "42%",
          style: {
            text: fmtInt(verifiedCount),
            fill: "#e6e6e6",
            font: "bold 1.6rem system-ui, sans-serif",
          },
        },
        {
          type: "text",
          right: 12,
          top: "60%",
          style: {
            text: "units",
            fill: "#9a9a9a",
            font: "11px system-ui, sans-serif",
          },
        },
      ],
      series: [
        {
          type: "bar",
          data: buckets.map((b) => ({
            value: b.value,
            itemStyle: { color: b.color },
          })),
          label: { show: true, position: "top", formatter: "{c}" },
          barWidth: "50%",
        },
      ],
    },
    { notMerge: true },
  );
}

function render(stats) {
  latestStats = stats;
  window.__simarisStats = stats;
  const hideSuperseded =
    document.getElementById("hide-superseded")?.checked ?? false;
  renderTiles(stats, hideSuperseded);
  renderTypes(stats);
  renderTags(stats);
  renderConfidence(stats);
}

async function loadStats() {
  setStatus("loading /api/stats …");
  try {
    const stats = await fetchJson("/api/stats");
    render(stats);
    setStatus(`/api/stats ok — ${shortTime()}`, "ok");
  } catch (err) {
    setStatus(`/api/stats failed: ${err.message}`, "error");
  }
}

function scheduleRefresh() {
  if (refreshTimer) clearInterval(refreshTimer);
  refreshTimer = setInterval(loadStats, REFRESH_MS);
}

// Reflow charts on window resize.
window.addEventListener("resize", () => {
  for (const c of Object.values(charts)) c?.resize();
});

// Re-render tiles client-side on toggle (no extra fetch needed for v1).
document.getElementById("hide-superseded")?.addEventListener("change", () => {
  if (latestStats) {
    const hide = document.getElementById("hide-superseded").checked;
    renderTiles(latestStats, hide);
  }
});

document.getElementById("refresh-now")?.addEventListener("click", loadStats);

loadStats();
scheduleRefresh();
