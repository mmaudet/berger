-- Berger — schéma initial du sidecar SQLite (PRD §5.9).
-- Migration refinery V1, appliquée à l'ouverture de la base.

-- Comptes mail synchronisés via Bichon.
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    bichon_account_id TEXT NOT NULL,
    last_cursor TEXT,
    last_polled_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Un mail traité = une ligne. Source de vérité de l'idempotence.
CREATE TABLE processed_messages (
    message_id TEXT PRIMARY KEY,
    account_id INTEGER REFERENCES accounts(id),
    bichon_uri TEXT,
    subject TEXT,
    from_email TEXT,
    from_name TEXT,
    date TIMESTAMP,
    processed_at TIMESTAMP NOT NULL,
    berger_version TEXT NOT NULL,
    config_hash TEXT NOT NULL
);
CREATE INDEX idx_processed_account_date ON processed_messages(account_id, date);

-- Tags appliqués (1 mail -> N tags).
CREATE TABLE applied_tags (
    message_id TEXT REFERENCES processed_messages(message_id),
    tag TEXT NOT NULL,
    applied_at TIMESTAMP NOT NULL,
    PRIMARY KEY (message_id, tag)
);
CREATE INDEX idx_tags_tag ON applied_tags(tag);

-- Filtres déclencheurs (traçabilité : pourquoi ce tag ?).
CREATE TABLE filter_matches (
    id INTEGER PRIMARY KEY,
    message_id TEXT REFERENCES processed_messages(message_id),
    filter_type TEXT NOT NULL,
    filter_name TEXT NOT NULL,
    details_json TEXT
);

-- Décisions LLM (audit + cache + métriques coût).
CREATE TABLE llm_decisions (
    id INTEGER PRIMARY KEY,
    message_id TEXT REFERENCES processed_messages(message_id),
    model TEXT NOT NULL,
    prompt_hash TEXT NOT NULL,
    prompt_text TEXT NOT NULL,
    response_json TEXT NOT NULL,
    tokens_input INTEGER,
    tokens_output INTEGER,
    latency_ms INTEGER,
    cost_usd REAL,
    called_at TIMESTAMP NOT NULL
);
CREATE INDEX idx_llm_cache ON llm_decisions(message_id, prompt_hash);

-- Actions IMAP exécutées.
CREATE TABLE executed_actions (
    id INTEGER PRIMARY KEY,
    message_id TEXT REFERENCES processed_messages(message_id),
    action_type TEXT NOT NULL,
    target TEXT,
    imap_response TEXT,
    succeeded BOOLEAN NOT NULL,
    error TEXT,
    executed_at TIMESTAMP NOT NULL
);

-- Webhooks émis.
CREATE TABLE webhook_emissions (
    id INTEGER PRIMARY KEY,
    message_id TEXT REFERENCES processed_messages(message_id),
    webhook_name TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    http_status INTEGER,
    attempts INTEGER NOT NULL,
    succeeded BOOLEAN NOT NULL,
    emitted_at TIMESTAMP NOT NULL,
    completed_at TIMESTAMP
);
