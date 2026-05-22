# Operations

This document covers running Berger in production: deployment with Docker
and with systemd, backing up the SQLite sidecar, reading the logs, and the
metrics Berger exposes.

## Contents

- [What Berger is, operationally](#what-berger-is-operationally)
- [Deployment with Docker](#deployment-with-docker)
- [Deployment with systemd](#deployment-with-systemd)
- [Configuration and secrets](#configuration-and-secrets)
- [The SQLite sidecar](#the-sqlite-sidecar)
- [Backups](#backups)
- [Logs](#logs)
- [Metrics and monitoring](#metrics-and-monitoring)
- [The WebUI](#the-webui)
- [Upgrades](#upgrades)
- [Resetting Berger](#resetting-berger)

## What Berger is, operationally

Berger is a single static Rust binary. It:

- runs one process, with one background task for the WebUI;
- polls every configured account once every **5 minutes**, then sleeps;
- keeps **all** of its state in one SQLite file (the *sidecar*);
- reads its YAML configuration **once, at startup** — there is no hot reload
  and no `SIGHUP` handler; a config change needs a restart;
- writes structured JSON logs to **stdout**;
- never phones home — it makes no network call other than to Bichon, the LLM
  endpoint and the webhooks you declared.

A per-account or per-message failure is logged and skipped; it does not stop
the daemon. The process is expected to run unattended for long stretches.

## Deployment with Docker

The repository ships a multi-stage [`Dockerfile`](../Dockerfile) and a
[`docker-compose.yml`](../docker-compose.yml). This is the recommended way
to run Berger.

```sh
git clone https://github.com/mmaudet/berger.git
cd berger

cp berger.example.yaml berger.yaml      # then edit it
$EDITOR berger.yaml

# Secrets go in a gitignored .env beside docker-compose.yml.
cat > .env <<'EOF'
BICHON_API_KEY=...
LINAGORA_IMAP_PASSWORD=...
MISTRAL_API_KEY=...
EOF

docker compose up --build -d            # build the image and start the daemon
docker compose logs -f                  # follow the JSON logs
```

What the compose file sets up:

- `./berger.yaml` is mounted **read-only** at `/etc/berger/berger.yaml`.
- The sidecar lives on a named volume `berger-data` mounted at `/data`,
  which is the container's working directory — `berger.yaml`'s
  `database.path: berger.db` therefore lands on that volume and survives
  restarts and image upgrades.
- The WebUI is published on `127.0.0.1:7000` only. Berger's WebUI has **no
  authentication**; put a reverse proxy with auth in front of it before
  exposing it beyond localhost.
- `restart: unless-stopped` brings the daemon back after a crash or a host
  reboot.

To run a one-off subcommand against the same configuration:

```sh
docker compose run --rm berger status   --config /etc/berger/berger.yaml
docker compose run --rm berger dry-run  --config /etc/berger/berger.yaml
docker compose run --rm berger explain '<msg-id@example.com>' \
  --config /etc/berger/berger.yaml
```

Building the image directly, without compose:

```sh
docker build -t berger:0.1.0 .
```

The image is a slim Debian base carrying only the binary and the CA
certificates `reqwest` needs for HTTPS; SQLite is statically linked.

## Deployment with systemd

To run Berger straight on a host, build the release binary and supervise it
with systemd.

Build (needs a stable Rust toolchain, edition 2024):

```sh
cargo build --release
sudo install -m 0755 target/release/berger /usr/local/bin/berger
```

Lay out the runtime files:

```sh
sudo useradd --system --home /var/lib/berger --shell /usr/sbin/nologin berger
sudo mkdir -p /etc/berger /var/lib/berger
sudo install -m 0640 -o berger -g berger berger.yaml /etc/berger/berger.yaml
```

In `/etc/berger/berger.yaml`, point the sidecar at the state directory:

```yaml
database:
  path: "/var/lib/berger/berger.db"
```

Put the secrets in an environment file readable only by root and the service
user — these become the `${VAR}` values the config interpolates:

```sh
sudo tee /etc/berger/berger.env >/dev/null <<'EOF'
BICHON_API_KEY=...
LINAGORA_IMAP_PASSWORD=...
MISTRAL_API_KEY=...
EOF
sudo chmod 0640 /etc/berger/berger.env
sudo chown root:berger /etc/berger/berger.env
```

Create `/etc/systemd/system/berger.service`:

```ini
[Unit]
Description=Berger — email triage daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=berger
Group=berger
ExecStart=/usr/local/bin/berger run --config /etc/berger/berger.yaml
EnvironmentFile=/etc/berger/berger.env
WorkingDirectory=/var/lib/berger
Restart=on-failure
RestartSec=10s

# Logs go to the journal as structured JSON.
StandardOutput=journal
StandardError=journal

# Hardening — Berger only needs to write its state directory.
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/berger

[Install]
WantedBy=multi-user.target
```

Enable and start it:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now berger.service
sudo journalctl -u berger -f
```

Because there is no hot reload, apply a config change with
`sudo systemctl restart berger`.

## Configuration and secrets

Berger reads `berger.yaml` once at startup. Secrets must **not** be written
into it — use `${VAR}` placeholders, which Berger substitutes from the
process environment (see [`yaml.md`](yaml.md#environment-variable-interpolation)).

- **Docker:** the `.env` file, loaded by `docker compose`.
- **systemd:** the `EnvironmentFile`.

An unset `${VAR}` is a fatal startup error — Berger never substitutes an
empty value. Berger's own types redact secrets from logs and debug output;
keep your `.env` / `.env`-style files out of version control (`.gitignore`
already excludes `*.env`).

## The SQLite sidecar

The sidecar is one file — `berger.db` by default — and it is the only state
Berger keeps. It holds seven tables: `accounts`, `processed_messages`,
`applied_tags`, `filter_matches`, `llm_decisions`, `executed_actions`,
`webhook_emissions`.

Operational facts:

- **WAL mode** is enabled automatically. Alongside `berger.db` you will see
  `berger.db-wal` and `berger.db-shm` — they are part of the database; do
  not delete them while Berger runs.
- **Schema migrations** run at every startup. They are embedded in the
  binary and idempotent — reopening an up-to-date database is a no-op.
- **No automatic purge.** History accumulates; it is deliberately kept for
  tuning prompts and debugging false positives. Expect roughly 200–500 MB
  per year at ~250 messages/day. Revisit only if it grows past a few GB.
- **`:memory:`** as `database.path` gives an ephemeral database — useful for
  testing, useless for a real deployment (all state is lost on exit).

Inspect it without stopping Berger:

```sh
berger status  --config berger.yaml             # counters and success rates
berger explain '<msg-id>' --config berger.yaml  # one message's full chain
```

## Backups

Berger does not back itself up. Because the sidecar is a single SQLite file
in WAL mode, a consistent backup can be taken **while Berger is running** —
use SQLite's online backup, never a plain `cp` (a plain copy can catch the
WAL mid-write).

A daily cron job, keeping 30 days:

```sh
#!/bin/sh
# /etc/cron.daily/berger-backup
set -eu
DB=/var/lib/berger/berger.db
DEST=/var/backups/berger
mkdir -p "$DEST"
sqlite3 "$DB" ".backup '$DEST/berger-$(date +%F).db'"
# Drop backups older than 30 days.
find "$DEST" -name 'berger-*.db' -mtime +30 -delete
```

`.backup` produces a single self-contained file with no WAL/SHM companions.
To restore, stop Berger, replace `berger.db` with a backup (and remove any
stale `berger.db-wal` / `berger.db-shm`), then start Berger again.

In Docker, run the same `sqlite3 ... .backup` inside the container or against
the `berger-data` volume.

## Logs

Berger logs to **stdout** as structured JSON, one object per line, via
`tracing`. The default level is `INFO`. Each line carries structured fields
— `account`, `message_id`, `webhook`, `folder`, and so on — so the log is
easy to filter and ship.

The level is controlled by the **`RUST_LOG`** environment variable
(standard `tracing` / `env-logger` syntax):

```sh
RUST_LOG=debug   berger run        # verbose, whole process
RUST_LOG=berger=debug,warn         # debug for berger, warn elsewhere
```

Set it in the `.env` file (Docker) or the `EnvironmentFile` (systemd).

Log levels in practice:

- `INFO` — daemon start, each poll cycle's summary, successful webhook
  delivery.
- `WARN` — a missing `Berger/*` folder recreated (a user deleted it); a
  `copy_to`/`move_to` conflict resolved in favour of `move_to`; an LLM call
  that failed and fell over to the `llm_error` tag.
- `ERROR` — a poll cycle that failed; a message that could not be processed;
  a webhook abandoned after its retry budget.

Reading them:

```sh
docker compose logs -f                    # Docker
sudo journalctl -u berger -f              # systemd
sudo journalctl -u berger -p warning      # warnings and errors only
```

Pipe the stream into `jq` for ad-hoc filtering, or into a log aggregator —
the JSON is structured for exactly that.

## Metrics and monitoring

Berger has **no Prometheus endpoint** at v1 (it is on the post-v1 roadmap).
Monitoring is done two ways:

1. **`berger status`** — a point-in-time snapshot of the sidecar: messages
   processed in total / last 24h / last 7d, cumulative LLM token counts and
   USD cost, IMAP-action success and failure counts, webhook success and
   failure counts, and per-table row counts. Scriptable for an external
   check:

   ```sh
   berger status --config /etc/berger/berger.yaml
   ```

2. **The logs** — the per-cycle `INFO` summary line tells you each poll
   completed and how many messages it triaged or skipped. An alert on the
   *absence* of that line, or on a rising rate of `ERROR` lines, is a good
   liveness and health signal.

A simple health check is "did a poll-cycle summary appear in the last ~10
minutes?" — the poll interval is 5 minutes, so two missed cycles means
something is wrong.

The WebUI `/` page renders the same statistics as `berger status`, for a
human glance.

## The WebUI

`berger run` serves a WebUI on **port 7000** (fixed) as a background task in
the same process. Four pages:

| Path | Content |
|---|---|
| `/` | Statistics — processed counts, LLM cost, webhook tallies. |
| `/recent` | The most recently triaged messages, with their tags and actions. |
| `/explain/<id>` | The full decision chain for one message. |
| `/config` | The active configuration, read-only, with secrets redacted. |

The WebUI has **no authentication**. Bind it to localhost (the supplied
`docker-compose.yml` does) and, if you need remote access, place it behind a
reverse proxy that adds TLS and auth. If the WebUI fails to bind its port,
it is logged and the daemon keeps triaging — the WebUI is not load-bearing.

## Upgrades

Upgrading Berger:

1. Back up the sidecar (see [Backups](#backups)).
2. Replace the binary or pull the new image.
3. Restart. Schema migrations, if any, run automatically at startup against
   the existing sidecar — no manual migration step.

   - **Docker:** `docker compose up --build -d` (or pull and `up -d`).
   - **systemd:** install the new binary, then `sudo systemctl restart
     berger`.

There is no downtime concern beyond the restart itself: a message in flight
is simply re-polled on the next cycle, and the idempotency ledger prevents
any double processing.

## Resetting Berger

To make Berger forget everything and start fresh:

1. Stop the daemon.
2. Delete the sidecar — `berger.db` **and** its `berger.db-wal` and
   `berger.db-shm` companions.
3. Optionally delete the `Berger/*` folders on the IMAP server, if you also
   want the foldering gone. Berger recreates them as new mail is triaged.
4. Start the daemon. A fresh sidecar is created; polling anchors at "now" —
   Berger does not back-fill history.

Deleting the sidecar alone resets Berger's memory but leaves the existing
`Berger/*` folders in place; that is fine — Berger will simply add to them.
