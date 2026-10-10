-- Providers and combos are either `chat` (OpenAI-compatible) or `decision`
-- (TypeSafe System One compatible: typed Choice / Score / Noul questions).
ALTER TABLE providers ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat' CHECK (kind IN ('chat', 'decision'));
ALTER TABLE combos ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat' CHECK (kind IN ('chat', 'decision'));
