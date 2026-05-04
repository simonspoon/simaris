# tools/ — out-of-band utilities

Operational scripts that live outside the Rust workspace. Use when the
in-process Rust path hits a known issue.

## `direct_backfill_ollama.py`

Direct-write Python fallback for the bge-m3 embedding backfill. Issues
sequential HTTP POSTs to `http://localhost:11434/api/embeddings` and writes
embeddings into a sqlite-vec virtual table — bypassing the simaris EMBED_CMD
subprocess pipeline.

**When to use.** When the Rust subprocess pipe-buffer deadlock recurs during
full-corpus backfill (observed M3.3 prep, recovered in M3-redo-2). Symptom:
backfill hangs after a few hundred rows; the Rust embedder process and
fastembed/ollama child processes deadlock waiting on each other's pipes.

**Reference.** `simaris-m3-redo-2-verdict-2026-05-04` documents the deadlock
and the direct-write workaround. The Rust path
(`simaris-vec::embed::OllamaEmbedClient`) is preferred when functional;
this Python script is the documented fallback.
