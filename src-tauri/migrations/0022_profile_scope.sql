-- A profile is a separate job search (semiconductor, data science, ...). Every source belongs to the
-- profile it was added under, and every application to the profile it was saved in, so switching
-- profile switches what the Sources, Jobs and Applications tabs show and what an update reads.
-- NULL means "belongs to no profile": shown only when no profile is chosen ("All profiles").
-- Existing rows are assigned at startup from the company catalog (Database::scope_to_profiles),
-- which lives in Rust, not here.
ALTER TABLE sources ADD COLUMN profile_id TEXT;
ALTER TABLE applications ADD COLUMN profile_id TEXT;
CREATE INDEX IF NOT EXISTS sources_profile_idx ON sources(profile_id);
CREATE INDEX IF NOT EXISTS applications_profile_idx ON applications(profile_id);
