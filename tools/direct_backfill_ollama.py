#!/usr/bin/env python3
"""Direct-write bge-m3 backfill (deadlock workaround fallback).

Bypasses the simaris EMBED_CMD subprocess pipeline. Issues sequential HTTP
POSTs to a local ollama instance and writes f32 vectors into a sqlite-vec
virtual table.

Use only when the Rust path (simaris-vec::embed::OllamaEmbedClient) hits the
pipe-buffer deadlock observed in M3.3 prep. Reference:
simaris-m3-redo-2-verdict-2026-05-04.

Requirements:
    - ollama running locally with bge-m3 pulled (`ollama pull bge-m3`)
    - sqlite-vec extension loadable from python (e.g. `pip install sqlite-vec`)
    - sanctuary.db with units table populated; vec table created externally

Usage:
    ./direct_backfill_ollama.py \\
        --db ~/.simaris/sanctuary.db \\
        --table units_vec_bge \\
        --model bge-m3 \\
        --dim 1024
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import urllib.request
from pathlib import Path
from typing import Iterator

DEFAULT_OLLAMA_URL = "http://localhost:11434/api/embeddings"


def embed_one(url: str, model: str, prompt: str, timeout: float = 120.0) -> list[float]:
    body = json.dumps({"model": model, "prompt": prompt}).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    emb = data.get("embedding")
    if not emb:
        raise RuntimeError(f"empty embedding from ollama (model={model})")
    return emb


def units_to_embed(db_path: Path) -> Iterator[tuple[str, str]]:
    import sqlite3

    con = sqlite3.connect(str(db_path))
    try:
        cur = con.execute(
            "SELECT id, content FROM units WHERE archived = 0 ORDER BY id"
        )
        for row in cur:
            yield row[0], row[1]
    finally:
        con.close()


def write_vec(db_path: Path, table: str, unit_id: str, vec: list[float]) -> None:
    import sqlite3
    import sqlite_vec  # type: ignore

    con = sqlite3.connect(str(db_path))
    try:
        con.enable_load_extension(True)
        sqlite_vec.load(con)
        blob = struct.pack(f"{len(vec)}f", *vec)
        con.execute(
            f"INSERT OR REPLACE INTO {table}(rowid, embedding) "
            f"SELECT rowid, ? FROM units WHERE id = ?",
            (blob, unit_id),
        )
        con.commit()
    finally:
        con.close()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--db", type=Path, required=True)
    ap.add_argument("--table", default="units_vec_bge")
    ap.add_argument("--model", default="bge-m3")
    ap.add_argument("--dim", type=int, default=1024)
    ap.add_argument("--url", default=DEFAULT_OLLAMA_URL)
    ap.add_argument("--limit", type=int, default=None)
    args = ap.parse_args()

    if not args.db.exists():
        print(f"db not found: {args.db}", file=sys.stderr)
        return 2

    n = 0
    for unit_id, content in units_to_embed(args.db):
        if args.limit is not None and n >= args.limit:
            break
        vec = embed_one(args.url, args.model, content)
        if len(vec) != args.dim:
            print(
                f"WARN: dim mismatch for {unit_id}: got {len(vec)}, want {args.dim}",
                file=sys.stderr,
            )
        write_vec(args.db, args.table, unit_id, vec)
        n += 1
        if n % 50 == 0:
            print(f"embedded {n} rows", file=sys.stderr)

    print(f"done: {n} rows", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
