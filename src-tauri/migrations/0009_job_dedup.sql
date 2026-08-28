ALTER TABLE jobs ADD COLUMN dedupe_fingerprint TEXT;
CREATE INDEX IF NOT EXISTS jobs_requisition_idx ON jobs(requisition_id) WHERE requisition_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS jobs_fingerprint_idx ON jobs(dedupe_fingerprint);
CREATE TABLE IF NOT EXISTS job_aliases (
  id TEXT PRIMARY KEY NOT NULL,
  canonical_job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  alias_job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  reason TEXT NOT NULL,
  created_at TEXT NOT NULL,
  removed_at TEXT,
  UNIQUE(canonical_job_id,alias_job_id)
) STRICT;
CREATE TABLE IF NOT EXISTS duplicate_candidates (
  id TEXT PRIMARY KEY NOT NULL,
  left_job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  right_job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  method TEXT NOT NULL CHECK(method IN ('exact','fuzzy')),
  score REAL,
  status TEXT NOT NULL DEFAULT 'suggested' CHECK(status IN ('suggested','merged','dismissed','unmerged')),
  evidence_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  decided_at TEXT,
  UNIQUE(left_job_id,right_job_id,method)
) STRICT;
CREATE TABLE IF NOT EXISTS duplicate_merge_audits (
  id TEXT PRIMARY KEY NOT NULL,
  canonical_job_id TEXT NOT NULL,
  merged_job_id TEXT NOT NULL,
  snapshot_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  undone_at TEXT,
  UNIQUE(canonical_job_id,merged_job_id,undone_at)
) STRICT;
CREATE INDEX IF NOT EXISTS duplicate_candidates_status_idx ON duplicate_candidates(status,created_at DESC);
