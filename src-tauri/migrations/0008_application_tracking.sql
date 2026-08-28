ALTER TABLE applications ADD COLUMN recruiter_name TEXT;
ALTER TABLE applications ADD COLUMN recruiter_email TEXT;
ALTER TABLE applications ADD COLUMN recruiter_phone TEXT;
ALTER TABLE applications ADD COLUMN source_attribution TEXT;
ALTER TABLE applications ADD COLUMN rejection_category TEXT;
ALTER TABLE applications ADD COLUMN withdrawn_reason TEXT;
ALTER TABLE applications ADD COLUMN accepted_at TEXT;

ALTER TABLE application_events ADD COLUMN reason TEXT;

ALTER TABLE notes ADD COLUMN updated_at TEXT;
ALTER TABLE notes ADD COLUMN deleted_at TEXT;
UPDATE notes SET updated_at=created_at WHERE updated_at IS NULL;

ALTER TABLE application_documents ADD COLUMN document_type TEXT NOT NULL DEFAULT 'other';
ALTER TABLE application_documents ADD COLUMN mime_type TEXT NOT NULL DEFAULT 'application/octet-stream';
ALTER TABLE application_documents ADD COLUMN size INTEGER NOT NULL DEFAULT 0;
ALTER TABLE application_documents ADD COLUMN event_id TEXT REFERENCES application_events(id) ON DELETE SET NULL;
UPDATE application_documents SET document_type=kind, size=length(content);

ALTER TABLE interviews ADD COLUMN outcome TEXT;
