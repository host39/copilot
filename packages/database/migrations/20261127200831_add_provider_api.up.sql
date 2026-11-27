-- add_provider_api: which OpenAI API an `openai` connection's answers use (docs/design/copilot.md,
-- Providers and Models). `chat_completions` is `POST {base}/chat/completions`, `responses` is
-- `POST {base}/responses` with `store: false`. `anthropic` connections have none (NULL).
--
-- Existing `openai` preset connections move to the Responses API, the preset's new default
-- (OpenAI's newest models call tools and summarize their reasoning only there); every other
-- `openai` connection keeps Chat Completions. Models detected while their connection used Chat
-- Completions stored `tools` off for `gpt-6*` under the `openai` preset (a lock that no longer
-- applies there, and detection is now stored before locks), so those flags go back to what
-- OpenAI's model list reports: nothing, which defaults to on.

ALTER TABLE providers
  ADD COLUMN api text CHECK (api IN ('chat_completions', 'responses'));

UPDATE providers
   SET api = CASE WHEN preset = 'openai' THEN 'responses' ELSE 'chat_completions' END
 WHERE kind = 'openai';

ALTER TABLE providers
  ADD CONSTRAINT providers_api_kind CHECK ((kind = 'openai') = (api IS NOT NULL));

UPDATE models AS m
   SET capabilities = jsonb_set(m.capabilities, '{tools}', 'true'::jsonb)
  FROM providers AS p
 WHERE p.id = m.provider_id
   AND p.user_id = m.user_id
   AND p.kind = 'openai'
   AND p.preset = 'openai'
   AND lower(m.remote_id) LIKE 'gpt-6%';
