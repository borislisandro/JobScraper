PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL, updated_at TEXT NOT NULL) STRICT;

CREATE TABLE IF NOT EXISTS sources (
  id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, base_url TEXT NOT NULL, adapter_id TEXT NOT NULL,
  adapter_version TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
  kind TEXT NOT NULL DEFAULT 'active' CHECK(kind IN ('active','reference')),
  disabled_reason TEXT, robots_override INTEGER NOT NULL DEFAULT 0 CHECK(robots_override IN (0,1)),
  robots_acknowledged_at TEXT, allow_private_network INTEGER NOT NULL DEFAULT 0 CHECK(allow_private_network IN (0,1)),
  last_success_at TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS source_configs (
  id TEXT PRIMARY KEY NOT NULL, source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
  config_json TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS source_configs_source_idx ON source_configs(source_id);

CREATE TABLE IF NOT EXISTS scrape_runs (
  id TEXT PRIMARY KEY NOT NULL, source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE RESTRICT,
  mode TEXT NOT NULL CHECK(mode IN ('test','scrape')), status TEXT NOT NULL CHECK(status IN ('running','completed','failed','cancelled')),
  complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1)), discovered_count INTEGER NOT NULL DEFAULT 0,
  persisted_count INTEGER NOT NULL DEFAULT 0, failure_code TEXT, diagnostics TEXT, started_at TEXT NOT NULL, finished_at TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS scrape_run_events (
  id TEXT PRIMARY KEY NOT NULL, run_id TEXT NOT NULL REFERENCES scrape_runs(id) ON DELETE CASCADE,
  level TEXT NOT NULL, event_type TEXT NOT NULL, payload_json TEXT NOT NULL, created_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS jobs (
  id TEXT PRIMARY KEY NOT NULL, source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE RESTRICT,
  external_id TEXT, canonical_url TEXT, apply_url TEXT, requisition_id TEXT, title TEXT NOT NULL, company TEXT NOT NULL,
  location TEXT, work_mode TEXT, description_text TEXT NOT NULL, description_html TEXT, posted_at TEXT, closing_at TEXT,
  salary_min REAL, salary_max REAL, salary_currency TEXT, salary_period TEXT, salary_confidence TEXT,
  seniority TEXT, skills_json TEXT NOT NULL DEFAULT '[]', availability TEXT NOT NULL DEFAULT 'active' CHECK(availability IN ('active','possibly_closed','closed','archived')),
  missing_full_runs INTEGER NOT NULL DEFAULT 0, content_hash TEXT NOT NULL, provenance_json TEXT NOT NULL DEFAULT '{}',
  extraction_at TEXT NOT NULL, adapter_version TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX IF NOT EXISTS jobs_external_idx ON jobs(source_id, external_id) WHERE external_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS jobs_canonical_idx ON jobs(canonical_url) WHERE canonical_url IS NOT NULL;
CREATE INDEX IF NOT EXISTS jobs_source_idx ON jobs(source_id, updated_at DESC);
CREATE TABLE IF NOT EXISTS job_occurrences (id TEXT PRIMARY KEY NOT NULL, job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, run_id TEXT NOT NULL REFERENCES scrape_runs(id) ON DELETE CASCADE, seen_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS job_revisions (id TEXT PRIMARY KEY NOT NULL, job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, content_hash TEXT NOT NULL, snapshot_json TEXT NOT NULL, created_at TEXT NOT NULL) STRICT;
CREATE VIRTUAL TABLE IF NOT EXISTS jobs_fts USING fts5(title, company, location, description_text, content='jobs', content_rowid='rowid');
CREATE TRIGGER IF NOT EXISTS jobs_ai AFTER INSERT ON jobs BEGIN INSERT INTO jobs_fts(rowid,title,company,location,description_text) VALUES(new.rowid,new.title,new.company,coalesce(new.location,''),new.description_text); END;
CREATE TRIGGER IF NOT EXISTS jobs_ad AFTER DELETE ON jobs BEGIN INSERT INTO jobs_fts(jobs_fts,rowid,title,company,location,description_text) VALUES('delete',old.rowid,old.title,old.company,coalesce(old.location,''),old.description_text); END;
CREATE TRIGGER IF NOT EXISTS jobs_au AFTER UPDATE ON jobs BEGIN INSERT INTO jobs_fts(jobs_fts,rowid,title,company,location,description_text) VALUES('delete',old.rowid,old.title,old.company,coalesce(old.location,''),old.description_text); INSERT INTO jobs_fts(rowid,title,company,location,description_text) VALUES(new.rowid,new.title,new.company,coalesce(new.location,''),new.description_text); END;

CREATE TABLE IF NOT EXISTS personas (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, target_titles_json TEXT NOT NULL, include_keywords_json TEXT NOT NULL DEFAULT '[]', exclude_keywords_json TEXT NOT NULL DEFAULT '[]', location TEXT, work_mode TEXT, seniority TEXT, salary_min REAL, threshold REAL NOT NULL DEFAULT 60, unknown_policy TEXT NOT NULL DEFAULT 'pass' CHECK(unknown_policy IN ('pass','require_known')), resume_document_id TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS resume_documents (id TEXT PRIMARY KEY NOT NULL, persona_id TEXT REFERENCES personas(id) ON DELETE SET NULL, filename TEXT NOT NULL, path TEXT NOT NULL, extracted_text TEXT NOT NULL, mime_type TEXT NOT NULL, content_hash TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS persona_skills (id TEXT PRIMARY KEY NOT NULL, persona_id TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE, skill TEXT NOT NULL, confirmed INTEGER NOT NULL DEFAULT 0 CHECK(confirmed IN(0,1)), required INTEGER NOT NULL DEFAULT 0 CHECK(required IN(0,1))) STRICT;
CREATE TABLE IF NOT EXISTS persona_filters (id TEXT PRIMARY KEY NOT NULL, persona_id TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE, filter_type TEXT NOT NULL, value TEXT NOT NULL, required INTEGER NOT NULL DEFAULT 0 CHECK(required IN(0,1))) STRICT;
CREATE TABLE IF NOT EXISTS embeddings (id TEXT PRIMARY KEY NOT NULL, owner_type TEXT NOT NULL, owner_id TEXT NOT NULL, model TEXT NOT NULL, dimensions INTEGER NOT NULL, content_hash TEXT NOT NULL, vector BLOB NOT NULL, created_at TEXT NOT NULL, UNIQUE(owner_type,owner_id,model,content_hash)) STRICT;
CREATE TABLE IF NOT EXISTS match_results (id TEXT PRIMARY KEY NOT NULL, job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, persona_id TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE, score REAL NOT NULL, eligible INTEGER NOT NULL CHECK(eligible IN(0,1)), algorithm_version TEXT NOT NULL, model_version TEXT NOT NULL, filter_decision_json TEXT NOT NULL, components_json TEXT NOT NULL, explanation_json TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, UNIQUE(job_id,persona_id,algorithm_version)) STRICT;
CREATE TABLE IF NOT EXISTS review_decisions (id TEXT PRIMARY KEY NOT NULL, job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, persona_id TEXT NOT NULL REFERENCES personas(id) ON DELETE CASCADE, status TEXT NOT NULL CHECK(status IN ('unseen','reviewing','shortlisted','dismissed')), reason TEXT, decided_at TEXT NOT NULL, UNIQUE(job_id,persona_id)) STRICT;

CREATE TABLE IF NOT EXISTS applications (id TEXT PRIMARY KEY NOT NULL, job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE RESTRICT, persona_id TEXT REFERENCES personas(id) ON DELETE SET NULL, current_stage TEXT NOT NULL CHECK(current_stage IN ('planned','applied','screening','interviewing','offer','accepted','rejected','withdrawn')), recruiter TEXT, rejection_reason TEXT, applied_at TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS application_events (id TEXT PRIMARY KEY NOT NULL, application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE, event_type TEXT NOT NULL, from_stage TEXT, to_stage TEXT, occurred_at TEXT NOT NULL, payload_json TEXT NOT NULL DEFAULT '{}') STRICT;
CREATE TABLE IF NOT EXISTS interviews (id TEXT PRIMARY KEY NOT NULL, application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE, stage TEXT NOT NULL, scheduled_at TEXT NOT NULL, completed_at TEXT, notes TEXT, created_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS notes (id TEXT PRIMARY KEY NOT NULL, application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE, body TEXT NOT NULL, created_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS application_documents (id TEXT PRIMARY KEY NOT NULL, application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE, kind TEXT NOT NULL, filename TEXT NOT NULL, content BLOB NOT NULL, sha256 TEXT NOT NULL, created_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS reminders (id TEXT PRIMARY KEY NOT NULL, application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE, reminder_type TEXT NOT NULL, due_at TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('pending','sent','cancelled','missed')), created_at TEXT NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS duplicate_groups (id TEXT PRIMARY KEY NOT NULL, primary_job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, member_job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE, reason TEXT NOT NULL, created_at TEXT NOT NULL, UNIQUE(primary_job_id,member_job_id)) STRICT;
