ALTER TABLE personas ADD COLUMN include_keyword_mode TEXT NOT NULL DEFAULT 'any' CHECK(include_keyword_mode IN ('any','all'));
