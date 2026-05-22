# Berger

[![CI](https://github.com/mmaudet/berger/actions/workflows/ci.yml/badge.svg)](https://github.com/mmaudet/berger/actions/workflows/ci.yml)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

Berger is an open-source email triage daemon written in Rust. It polls your
mail through the [Bichon](docs/bichon-setup.md) archiver, tags each message
with declarative native filters and a pluggable LLM, and materialises the
result as IMAP folders that show up in every mail client. Anything beyond
plain triage — drafting replies, push notifications, delegation — is handed
off to external workflows over webhooks.

It is the *afew of 2026*: open-source, AGPL, server-agnostic, client-agnostic,
LLM-pluggable.

## How it works

```
  Mail servers ──IMAP──▶ Bichon ──REST──▶ Berger ──IMAP COPY/MOVE──▶ Berger/<tag>/ folders
  (Twake, Gmail, …)      (archive +        │                         (on the source server,
                          index)           │                          visible in every client)
                                           └──POST──▶ webhooks (n8n / Hermes / LinaTwin)
```

Berger never reads IMAP directly — every message comes through Bichon. It
writes back over IMAP only to move and flag messages, and it never deletes
mail or alters message content. All of its state lives in a single SQLite
sidecar file.

For each polled message Berger runs a fixed pipeline: an idempotency check
(by `Message-ID`), the native filters, the LLM classifier, a mapping from
the classification to tags, then the per-tag IMAP actions and webhooks.

## Status

Berger **v0.2.1** is the current release: v0.1.0 shipped the triage daemon,
and v0.2.0 added the read-only `berger scan` configuration bootstrapper. The
specification is in [`docs/PRD.md`](docs/PRD.md) and
[`docs/PRD-v1.1.md`](docs/PRD-v1.1.md); release notes are in the
[changelog](CHANGELOG.md).

## Quickstart

Berger ships as a Docker image and a `docker-compose.yml`. You need a
running Bichon instance and IMAP credentials for the accounts you want to
triage.

```sh
# 1. Get the source.
git clone https://github.com/mmaudet/berger.git
cd berger

# 2. Write your configuration.
cp berger.example.yaml berger.yaml
$EDITOR berger.yaml          # set the Bichon URL, accounts, filters, actions

# 3. Fill in the environment file (gitignored). Copy the template, then set
#    every value your berger.yaml references.
cp .env.example .env
$EDITOR .env

# 4. Build and start the daemon.
docker compose up --build -d

# 5. Follow the logs and open the WebUI.
docker compose logs -f       # structured JSON logs
open http://localhost:7000   # stats, recent messages, per-message explain
```

Before you point Berger at a real account, see what it *would* do without
touching anything:

```sh
docker compose run --rm berger dry-run --config /etc/berger/berger.yaml
```

The first real run uses `copy_to` only — nothing leaves your INBOX, nothing
is deleted, every action is reversible. Switch tags to `move_to` once you
trust the triage.

To run Berger outside Docker, see [`docs/ops.md`](docs/ops.md) for a
`cargo build --release` and a systemd unit.

## Bootstrapping a configuration with `berger scan`

Writing the `filters:` section from a blank page is the hard part.
`berger scan` does the first draft for you: it reads a mailbox **strictly
read-only** — no IMAP action, no LLM call, never a message body — measures
ten dimensions of it, and writes a suggested configuration you review
before using.

```sh
berger scan --since 150d     # analyse the last 150 days of every account
```

It writes a timestamped `berger-scan-<timestamp>.yaml` — a *suggestion*, it
never touches your real `berger.yaml` — plus a JSON copy and a text report.

**Factored suggestions.** The scan does not emit one rule per sender it
sees. It factors its findings into a handful of category-level rules: one
native `list_unsubscribe` rule for all newsletters, one `header_match` on
`List-Id` for all mailing lists, one `sender_in` list for notification
services, one for your two-way ("VIP") contacts, one `header_match` for
spam. Frequent senders and domains are reported but not turned into rules —
a frequent sender is not a triage category, and grouping domains into
themes (vendor, public sector, …) is a judgement left to you or the LLM.

**The process, end to end:**

1. **Scan** — `berger scan --since 150d`. Read-only; changes nothing.
2. **Review** — open `berger-scan-<timestamp>.yaml`. Every rule carries its
   evidence and a confidence score as comments.
3. **Merge** — copy the rules you keep into your `berger.yaml`, and add the
   matching `actions:` entries for their tags.
4. **Dry-run** — `berger dry-run` prints what those rules would tag,
   applying nothing.
5. **Activate** — `berger run` starts the daemon. The first real run uses
   `copy_to` only; switch tags to `move_to` once you trust the triage.

Full reference: [`docs/scan.md`](docs/scan.md).

## CLI

The single `berger` binary exposes six subcommands. Each reads the same
`berger.yaml` (`--config`, default `berger.yaml`).

| Command | What it does |
|---|---|
| `berger run` | Run the triage daemon: poll, filter, act, repeat. Also serves the WebUI on port 7000. |
| `berger dry-run` | Run one poll cycle applying **no** IMAP action and recording nothing — print the tags and actions Berger would apply. Native filters only. |
| `berger scan` | Analyse a mailbox **read-only** and suggest a starting `berger.yaml` — no IMAP action, no LLM call, never a message body. See [Bootstrapping](#bootstrapping-a-configuration-with-berger-scan). |
| `berger explain <message-id>` | Reconstruct the full decision chain of one processed message: tags, the filters and LLM decision behind them, the IMAP actions, the webhooks. |
| `berger status` | Print a health and statistics summary of the sidecar: messages processed, LLM cost, IMAP-action and webhook success rates, table counts. |
| `berger export-thunderbird` | Export the `actions:` configuration as a Mozilla Thunderbird `msgFilterRules.dat` ruleset, so the same foldering can run client-side. |

`berger export-thunderbird` takes `--account <name>` (defaults to the first
configured account, since Thunderbird keeps one ruleset per account) and
`--output <file>` (defaults to stdout). The rules match against the
`X-Berger-Tags` header — Berger records its tags in the sidecar under that
name and never injects it into the mail itself.

## Configuration

Berger is configured by a single YAML file. The example at
[`berger.example.yaml`](berger.example.yaml) is a complete, annotated
reference covering all four native filters, all five action primitives, the
LLM section and three webhooks.

The full reference documentation:

- [`docs/yaml.md`](docs/yaml.md) — every section and field of `berger.yaml`.
- [`docs/scan.md`](docs/scan.md) — the read-only `berger scan` analysis and
  the configuration it suggests.
- [`docs/webhooks.md`](docs/webhooks.md) — the canonical webhook payload and
  example n8n workflows.
- [`docs/bichon-setup.md`](docs/bichon-setup.md) — configuring the upstream
  Bichon archiver.
- [`docs/ops.md`](docs/ops.md) — deployment, systemd, backups, logs, metrics.

Secrets are never written into the YAML: `${VAR}` placeholders are
substituted from the environment at startup.

## A note on consistency

Berger and Bichon reach consistency *eventually*, not synchronously. When
Berger moves a message, Bichon's index does not know until its next poll —
expect one to five minutes of lag between a Berger action and Bichon's view
of it. This is by design; Berger is a best-effort triage daemon, not a
transactional system.

`move_to` is opt-in for exactly this reason: it removes the message from the
INBOX, so a folder it has moved mail into should sit behind an IMAP trash
with server-side retention. `copy_to`, the non-destructive default, leaves
the original in place.

## Building from source

Berger is a single Rust crate. It needs a stable Rust toolchain (edition
2024).

```sh
cargo build --release          # binary at target/release/berger
cargo test                     # unit + integration tests
cargo clippy --all-targets --all-features -- -D warnings
```

Some integration tests run a disposable IMAP server and HTTP mocks in
containers, so Docker must be available to run the full test suite.

## Contributing

Berger is developed in the open. Contributions are welcome.

- **Issues and pull requests** go to
  [github.com/mmaudet/berger](https://github.com/mmaudet/berger).
- **Scope.** Berger v1 has a deliberately tight scope; see
  [`docs/PRD.md`](docs/PRD.md) §6 for what is explicitly out. Please open an
  issue to discuss a feature before sending a PR for it.
- **Commits.** Follow [Conventional Commits](https://www.conventionalcommits.org)
  (`feat:`, `fix:`, `docs:`, `test:`, `chore:`). Keep commits small and
  atomic — one intention each.
- **Before you push.** `cargo fmt --all`, `cargo clippy --all-targets
  --all-features -- -D warnings` and `cargo test` must all pass. CI runs the
  same checks.
- **Code style.** No `unsafe`. No `unwrap()` in production code without a
  `// SAFETY:` justification. `thiserror` for domain errors, `anyhow` only
  at the binary's edges. Structured `tracing` for logs.

By contributing you agree your contribution is licensed under the AGPLv3.

## Licence

Berger is distributed under the **GNU Affero General Public License v3.0 or
later** (`AGPL-3.0-or-later`). The full text is in [`LICENSE`](LICENSE).

The AGPL's network clause matters here: if you run a modified Berger as a
network service, you must offer its users the corresponding source.

Copyright (C) 2026 Michel-Marie Maudet
