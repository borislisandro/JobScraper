-- Source rows are reconciled in Database::install_starter_pack because only it
-- can distinguish exact, untouched legacy starter metadata from user content.
INSERT INTO schema_metadata(key, value) VALUES ('starter_pack_reconcile', '2026-08-28')
ON CONFLICT(key) DO UPDATE SET value=excluded.value;
