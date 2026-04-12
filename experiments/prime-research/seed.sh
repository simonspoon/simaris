#!/usr/bin/env bash
# Populate an isolated simaris store with a hand-crafted fixture for prime experiments.
# Ground truth: 5 domains × 7 units + 10 distractors = 45 units.
# Each task in tasks.txt should match exactly one domain's 7 units.
set -euo pipefail

SEED_HOME="${SEED_HOME:-/tmp/simaris-prime-experiment}"
export SIMARIS_HOME="$SEED_HOME"

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO_ROOT/target/release/simaris"

if [ ! -x "$BIN" ]; then
  echo "Building release binary..."
  (cd "$REPO_ROOT" && cargo build --release --quiet)
fi

rm -rf "$SEED_HOME"
mkdir -p "$SEED_HOME"

add() {
  local type="$1" tags="$2" content="$3"
  "$BIN" add "$content" --type "$type" --tags "$tags" --source fixture >/dev/null
}

echo "Seeding $SEED_HOME ..."

# ─── Domain 1: code-review ─────────────────────────────────────────
add aspect     "code-review,review,quality" \
  "Code Review Aspect. Disposition: skeptical, thorough, read-only. Default stance: code has bugs until proven otherwise. Read changed files in full, never just diff lines. Priorities: Security > Correctness > Performance > Style."
add procedure  "code-review,workflow" \
  "Code review workflow: (1) read full diff (2) read changed files in full (3) security scan (4) bug detection (5) performance review (6) test coverage check."
add procedure  "code-review,output-format" \
  "Code review output format: group findings as Critical/Warning/Info. Every finding needs a file:line reference. Omit empty severity sections."
add principle  "code-review,security" \
  "Security findings in code review are always Critical — never downgrade to Warning or Info."
add preference "code-review,preferences" \
  "Code review findings must include file:line references so reviewers can navigate directly."
add fact       "code-review,metrics" \
  "PRs with more than 500 lines changed have roughly 40% higher post-merge defect rates than smaller PRs."
add lesson     "code-review,lessons" \
  "A fresh sub-agent reviewing with no prior context finds more issues than inline review by the authoring agent — cold loading forces literal workflow adherence."

# ─── Domain 2: rust-release ────────────────────────────────────────
add aspect     "rust-release,release,engineering" \
  "Release Engineer Aspect. Disposition: meticulous, cautious, reproducible. Every release must be tagged, built from clean state, and verified before publication."
add procedure  "rust-release,cargo,workflow" \
  "Rust CLI release steps: (1) bump Cargo.toml version (2) cargo test (3) cargo build --release (4) git tag vX.Y.Z (5) git push --tags (6) gh release create."
add procedure  "rust-release,homebrew,brew" \
  "Update homebrew formula after release: compute sha256 of tarball, update formula with new version+hash, commit to tap, test install locally."
add principle  "rust-release,principles" \
  "Never cut a release from a dirty working tree. Commit or stash everything first."
add preference "rust-release,preferences" \
  "After cutting a release for a suite tool, install via brew upgrade/install, not cargo install — keeps all machines on the same bottled artifact."
add fact       "rust-release,cargo" \
  "Cargo.toml version must match the git tag for crates.io publish to succeed."
add lesson     "rust-release,lessons" \
  "Forgetting to push tags after creating them locally silently breaks CI release pipelines — they never see the tag."

# ─── Domain 3: solidjs-debug ───────────────────────────────────────
add aspect     "solidjs,debugging,ui" \
  "UI Debugger Aspect. Disposition: systematic, reproducible, first-principles. Always reproduce the bug locally before diagnosing. Trust nothing until observed."
add procedure  "solidjs,debugging,freeze" \
  "SolidJS freeze debug steps: (1) grep for components sharing the same local state (2) check for missing onCleanup on event listeners (3) audit effect dependencies (4) add debug logs at suspected hotspots."
add procedure  "solidjs,devtools" \
  "SolidJS browser devtools workflow: (1) open console (2) check for errors/warnings (3) record performance profile (4) inspect the reactivity graph."
add principle  "solidjs,debugging,principles" \
  "Always reproduce a UI bug locally before attempting a fix — fixing something you can't observe is gambling."
add preference "solidjs,debugging,preferences" \
  "Use a visible browser (not headless) for UI debugging — direct observation beats logs."
add fact       "solidjs,reactivity" \
  "SolidJS signals are eagerly evaluated and do not re-render the entire component, unlike React's useState."
add lesson     "solidjs,lessons,memory" \
  "Missing onCleanup on event listeners is the single biggest cause of SolidJS memory leaks and UI freezes."

# ─── Domain 4: db-migration ────────────────────────────────────────
add aspect     "migration,database,schema" \
  "Migration Aspect. Disposition: zero-downtime, reversible, backward-compatible. Every schema change must be rollback-safe and compatible with in-flight application versions."
add procedure  "migration,workflow" \
  "Schema migration steps: (1) write forward migration (2) write backward migration (3) test on dev db (4) apply to staging (5) apply to prod during low-traffic window."
add procedure  "migration,backfill" \
  "Adding a NOT NULL column to a large table: (1) add column as nullable (2) backfill existing rows in batches (3) add NOT NULL constraint in a follow-up migration."
add principle  "migration,principles" \
  "Never drop a column in the same migration as adding its replacement — old app versions still reading the old column will break."
add preference "migration,preferences" \
  "Run production migrations manually by an engineer, not via CI automation — too much blast radius for fully-automated application."
add fact       "migration,database,locking" \
  "Adding an index with ALTER TABLE on a large table blocks writes for the full duration in most SQL databases."
add lesson     "migration,lessons" \
  "A migration that assumed constant schema during deployment caused a multi-hour outage — always design migrations to be compatible with both old and new app versions."

# ─── Domain 5: rust-testing ────────────────────────────────────────
add aspect     "rust-testing,testing,quality" \
  "Test Author Aspect. Disposition: paranoid, thorough, boundary-obsessed. Every test verifies a specific claim. Coverage is not the goal — confidence is."
add procedure  "rust-testing,workflow" \
  "Rust test-driven workflow: (1) write failing test (2) make it pass (3) mutate production code to verify the test fails (4) refactor for clarity."
add procedure  "rust-testing,integration" \
  "Integration test setup in simaris-style Rust projects: use a TestEnv struct that creates a temp SIMARIS_HOME, cleaned up on Drop. Each test runs in isolation."
add principle  "rust-testing,principles" \
  "Tests must fail loudly when the thing they test breaks — silent passes mean the test asserts nothing."
add preference "rust-testing,preferences" \
  "Prefer cargo test over custom test harnesses for Rust projects — it integrates with cargo, clippy, and editor tooling for free."
add fact       "rust-testing,cargo" \
  "Rust tests run in parallel by default. Use --test-threads=1 for serial execution when tests share global state."
add lesson     "rust-testing,lessons" \
  "Property-based tests with proptest catch edge cases that example-based unit tests consistently miss — especially around empty inputs, unicode, and integer boundaries."

# ─── Distractors ───────────────────────────────────────────────────
add fact       "python,uv,tooling" \
  "Python uv replaces pip and venv with a single fast tool for dependency and virtualenv management."
add preference "javascript,pnpm,preferences" \
  "Use pnpm over npm for JavaScript projects — faster installs and stricter dependency resolution."
add fact       "git,merge,rebase" \
  "Git merge preserves branch history as-is; git rebase replays commits linearly onto a new base."
add procedure  "claude-code,hooks" \
  "Claude Code hooks fire on specific events like UserPromptSubmit and Stop. Configured via settings.json."
add fact       "warframe,trivia" \
  "Warframe frame names are drawn from Greek mythology, deities, and elemental concepts."
add principle  "scripting,preferences" \
  "For scripting tasks in this suite: Bash first, Python second (via uv), JavaScript last."
add fact       "sqlite,fts" \
  "SQLite FTS5 uses a lighter indexing scheme and supports more query syntax than FTS4."
add lesson     "kubernetes,deployment" \
  "Kubernetes is overkill for small projects — a single VM with systemd or a PaaS is usually simpler and cheaper."
add fact       "macos,system" \
  "macOS uses launchd for service management, not systemd."
add preference "performance,preferences" \
  "In hot paths, prefer stack allocation over heap allocation when object lifetimes permit."

count=$("$BIN" list --json | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')
echo "Seeded $count units into $SEED_HOME"
