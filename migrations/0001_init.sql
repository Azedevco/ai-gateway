-- Upstream OpenAI-compatible APIs.
CREATE TABLE providers (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,
    base_url    TEXT    NOT NULL,           -- e.g. https://api.deepseek.com/v1
    api_key     TEXT    NOT NULL DEFAULT '',
    enabled     INTEGER NOT NULL DEFAULT 1,
    timeout_secs INTEGER NOT NULL DEFAULT 120,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Models offered by a provider. `name` is the upstream model id.
CREATE TABLE models (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id INTEGER NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (provider_id, name)
);

-- A combo is a virtual model name; its entries are tried in position order (fallback).
CREATE TABLE combos (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,
    description TEXT    NOT NULL DEFAULT '',
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE combo_entries (
    combo_id    INTEGER NOT NULL REFERENCES combos(id) ON DELETE CASCADE,
    model_id    INTEGER NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (combo_id, model_id)
);

-- Client keys. Only the SHA-256 of the key is stored.
CREATE TABLE api_keys (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT    NOT NULL,
    key_hash     TEXT    NOT NULL UNIQUE,
    key_prefix   TEXT    NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_used_at TEXT
);

-- One row per client request.
CREATE TABLE requests (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at        TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    api_key_id        INTEGER REFERENCES api_keys(id) ON DELETE SET NULL,
    requested_model   TEXT    NOT NULL,
    model_id          INTEGER REFERENCES models(id) ON DELETE SET NULL,  -- model that answered
    model_name        TEXT,                                              -- kept if the model is deleted
    provider_name     TEXT,
    status            TEXT    NOT NULL,   -- ok | error
    http_status       INTEGER NOT NULL,
    attempts          INTEGER NOT NULL DEFAULT 1,
    stream            INTEGER NOT NULL DEFAULT 0,
    latency_ms        INTEGER NOT NULL,
    prompt_tokens     INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens      INTEGER NOT NULL DEFAULT 0,
    error             TEXT
);
CREATE INDEX requests_created_at ON requests (created_at);
CREATE INDEX requests_model_id ON requests (model_id);

-- One row per upstream call; a request with fallback has several.
CREATE TABLE attempts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id  INTEGER NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    model_id    INTEGER REFERENCES models(id) ON DELETE SET NULL,
    model_name  TEXT    NOT NULL,
    provider_name TEXT  NOT NULL,
    ok          INTEGER NOT NULL,
    http_status INTEGER,
    latency_ms  INTEGER NOT NULL,
    error       TEXT,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX attempts_request_id ON attempts (request_id);
CREATE INDEX attempts_created_at ON attempts (created_at);
