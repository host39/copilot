-- add_provider_api: revert. Detected `tools` flags the up migration turned back on stay on; a
-- refresh on Chat Completions locks them again at read time.

ALTER TABLE providers DROP CONSTRAINT providers_api_kind;

ALTER TABLE providers DROP COLUMN api;
