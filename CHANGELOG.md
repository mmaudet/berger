# Changelog

All notable changes to Berger are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-05-22

Adds `berger scan` — a strictly read-only inbox analysis that observes ten
dimensions of a mailbox and proposes a starting `berger.yaml`. The scan
applies no IMAP action, never calls the LLM, and never reads a message
body. The v0.1.0 triage daemon is unchanged.

### Added

#### `berger scan`
- New `scan` subcommand: a read-only analysis of recent mail, fetched from
  Bichon over a `--since` window, producing a report and suggested filter
  rules.
- Ten analysis dimensions — top senders, bidirectional contacts, sender
  domains, newsletters, mailing lists, notification services, spam
  signals, subject n-grams, dominant language, and hourly volume.
- Suggested rules are scored by a confidence formula and gated by
  `--min-evidence`; overlapping rules are consolidated so a typical
  message is tagged once, at most twice.
- Three output formats selected with `--format`: a text report, the
  suggested-configuration YAML, and a JSON document.
- `--save-report` persists a run to the new `scan_reports` table.

#### Persistence
- Migration V2 adds the `scan_reports` table.

### Documentation
- `docs/scan.md` — the scan command, its dimensions, output formats, the
  confidence formula, and its read-only guarantees.

## [0.1.0] - 2026-05-22

First public release. Berger triages email through the Bichon archiver: it
tags messages with native filters and a pluggable LLM, then materialises the
result as IMAP folders visible in every mail client. It never deletes mail,
never alters message content, and never phones home.

### Added

#### Ingestion
- Bichon REST client with incremental, date-watermarked polling.
- Source folders under `Berger/*` are skipped on read, so Berger never
  re-triages its own output.

#### Triage
- Four native filters: `list_unsubscribe`, `sender_in`, `subject_regex` and
  `header_match`.
- LLM classifier against any OpenAI-compatible chat endpoint, with a typed
  JSON schema and a per-message decision cache. Endpoint-agnostic: works
  with a hosted model such as Mistral Small 3 or a local Ollama model.
- Declarative mapping from an LLM classification to triage tags.

#### Actions
- IMAP action engine with five primitives: `copy_to`, `move_to`,
  `mark_seen`, `mark_flagged` and `webhook`.
- `ensure_folder_exists` recreates a user-deleted destination folder before
  each `copy_to` / `move_to`.
- Idempotence by `Message-ID`: a message submitted again is never acted on
  twice.

#### Webhooks
- `POST` webhooks carrying the canonical `berger.tag_applied` payload.
- Handlebars templating, retry with backoff, and an audit table recording
  every emission.

#### Interfaces
- CLI: `run`, `explain`, `status`, `dry-run` and `export-thunderbird`.
- Web UI on `:7000` with four pages: dashboard, recent activity, the
  per-message decision trace, and the loaded configuration.

#### Persistence & operations
- SQLite sidecar: seven tables, refinery migrations run at startup, WAL mode.
- Multi-account YAML configuration with `${VAR}` interpolation.
- Multi-stage Docker image and a `docker-compose.yml`.
- Reference documentation: `README`, `docs/yaml.md`, `docs/webhooks.md`,
  `docs/bichon-setup.md`, `docs/ops.md` and `berger.example.yaml`.

[0.2.0]: https://github.com/mmaudet/berger/releases/tag/v0.2.0
[0.1.0]: https://github.com/mmaudet/berger/releases/tag/v0.1.0
