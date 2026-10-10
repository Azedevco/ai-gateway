-- The kind moves from the provider to the model: one provider (same URL and key) can serve
-- chat models and decision models side by side.
ALTER TABLE models ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat' CHECK (kind IN ('chat', 'decision'));
UPDATE models SET kind = (SELECT p.kind FROM providers p WHERE p.id = models.provider_id);
-- providers.kind (from 0002) is no longer read.
