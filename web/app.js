// Shared fetch + status helpers for simaris admin pages.
//
// Used as an ES module by dashboard.js and units.js. No bundler.

/**
 * Fetch a JSON endpoint relative to the page origin.
 *
 * Throws on non-2xx or non-JSON responses; callers should catch and
 * surface via setStatus().
 */
export async function fetchJson(path, opts = {}) {
  const res = await fetch(path, {
    headers: { Accept: "application/json" },
    ...opts,
  });
  if (!res.ok) {
    throw new Error(`${res.status} ${res.statusText} — ${path}`);
  }
  const ctype = res.headers.get("content-type") || "";
  if (!ctype.includes("application/json")) {
    throw new Error(`expected JSON, got ${ctype || "unknown"} — ${path}`);
  }
  return res.json();
}

/**
 * Write a message to the status bar at the bottom of the page.
 *
 * `level` is one of "info" | "ok" | "error". The status element must
 * have id="status" in the page.
 */
export function setStatus(msg, level = "info") {
  const el = document.getElementById("status");
  if (!el) return;
  el.textContent = msg;
  el.classList.remove("status--ok", "status--error");
  if (level === "ok") el.classList.add("status--ok");
  if (level === "error") el.classList.add("status--error");
}

/**
 * Format an ISO timestamp as a short local time, for the status bar.
 */
export function shortTime(date = new Date()) {
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
