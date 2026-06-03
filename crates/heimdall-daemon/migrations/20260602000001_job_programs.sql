-- Per-job mapping for "the latest program bytes loaded onto the DUT".
-- Populated by the fuzz worker's program observer; consumed by the
-- /jobs/:id/disasm route as a fallback when the in-memory
-- LoadedProgramCache is cold (e.g. after a daemon restart).
--
-- One row per job; the worker upserts as iters advance, so the row
-- always reflects the most recent iter's program. The actual bytes
-- live in the BlobStore under `blob_id`; this table just tracks the
-- pointer plus enough metadata (kind, iter) for the disasm route to
-- assemble its response without round-tripping the JobKind.

CREATE TABLE IF NOT EXISTS job_programs (
    job_id      TEXT PRIMARY KEY NOT NULL,
    blob_id     TEXT NOT NULL,
    -- ArtifactKind serialized as kebab-case JSON tag, mirroring the
    -- on-the-wire representation (e.g. "raw-bytes", "elf-riscv").
    kind        TEXT NOT NULL,
    -- Iteration index for fuzz jobs (0-based). NULL for one-shot
    -- jobs whose program doesn't have an iter concept.
    iter        INTEGER,
    recorded_at TEXT NOT NULL
);
