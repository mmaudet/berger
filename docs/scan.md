# `berger scan` — initial inbox analysis

`berger scan` reads a mailbox and reports the patterns in it — the busiest
senders, the newsletters, the mailing lists, the spam already flagged
upstream — then proposes a starting `berger.yaml` you can review and merge.
It is the fastest way to bootstrap a configuration: instead of writing
filter rules from a blank page, you run one command and edit what it
suggests.

The scan is **strictly read-only**. It applies no IMAP action, never calls
the LLM, and never reads a message body. Running it cannot change anything
in your mailbox.

## Contents

- [What the scan does](#what-the-scan-does)
- [Quick start](#quick-start)
- [Command-line options](#command-line-options)
- [The ten dimensions](#the-ten-dimensions)
- [Output formats](#output-formats)
- [Suggestions and confidence](#suggestions-and-confidence)
- [Persisting a scan](#persisting-a-scan)
- [Guarantees](#guarantees)
- [The bidirectional dimension and Sent folders](#the-bidirectional-dimension-and-sent-folders)

## What the scan does

The scan fetches recent message envelopes from [Bichon](bichon-setup.md),
the upstream archiver, over a time window you choose. For inbox messages it
also downloads the raw message to read its technical headers (`List-Id`,
`List-Unsubscribe`, `X-Spam-Flag`, …) — headers only, never the body. It
then measures ten dimensions of the mailbox and writes a report together
with a suggested configuration.

Nothing is applied. The output is for you to read.

## Quick start

```
berger scan
```

With no options, this scans the last 30 days of every configured account,
prints a text report to the terminal, and writes the suggested
configuration (and a JSON copy) to timestamped files beside it.

To analyze a longer history — say, everything since the start of the year:

```
berger scan --since 150d
```

## Command-line options

| Option | Default | Meaning |
|---|---|---|
| `--config <path>` | `berger.yaml` | Configuration file — used for the Bichon connection and the account list. |
| `--since <Nd>` | `30d` | How far back to scan, as a whole number of days (e.g. `7d`, `90d`). |
| `--account <name>` | all accounts | Restrict the scan to one configured account by name. |
| `--format <fmt>` | `all` | `text`, `yaml`, `json`, or `all`. |
| `--output <path>` | timestamped | Output file path, for a single-format run. |
| `--min-evidence <N>` | `5` | Fewest messages a pattern needs before it becomes a suggestion. |
| `--save-report` | off | Also store the run in the sidecar's `scan_reports` table. |

## The ten dimensions

The scan measures ten dimensions of the mailbox (PRD v1.1 §4.2):

1. **Top senders** — the addresses that send you the most mail.
2. **Bidirectional contacts** — people you both receive from and write to. Needs a Sent folder (see below).
3. **Sender domains** — the busiest domains, across all of their addresses.
4. **Newsletters** — bulk senders that carry a `List-Unsubscribe` header, grouped by domain.
5. **Mailing lists** — discussion lists, identified by their `List-Id` header.
6. **Notification services** — automated senders such as CI, code hosting, and SaaS alerts.
7. **Spam signals** — messages already flagged upstream (`X-Spam-Flag`, a high `X-Spam-Score`, a DMARC failure).
8. **Subject patterns** — recurring 2- and 3-word phrases in subject lines, after stopwords are removed.
9. **Language** — the dominant language(s), detected from a sample of subject lines.
10. **Hourly volume** — how your mail is spread across the hours of the day.

## Output formats

`--format` selects what the scan writes:

- `text` — a human-readable report, printed to stdout.
- `yaml` — the suggested configuration, written to a file.
- `json` — the full scan (report and suggestions) as a JSON document, for third-party tools.
- `all` (default) — the text report on stdout, plus the YAML and JSON files.

For a single-format run, `--output` names the file. With `all` the scan
writes more than one file, so it always uses timestamped names
(`berger-scan-<timestamp>.yaml`, `berger-scan-<timestamp>.json`).

The suggested YAML is a `filters:` block of ordinary filter rules, each
annotated with its evidence and confidence as comments. Review it, then
merge the rules you want into your real `berger.yaml`. Nothing is applied
automatically.

## Suggestions and confidence

Each suggested rule carries a **confidence** score in `[0, 1]`, computed
from how much evidence backs it (PRD v1.1 §4.4):

```
confidence = min(1, ln(messages) / 4 + bidirectional_ratio × 0.3)
```

More messages, and a contact you also write to, raise the score.
`--min-evidence` sets the floor: a pattern seen fewer than that many times
produces no suggestion (default 5).

### One tag per message

The suggester only ever emits the two filter types the configuration
loader already understands — `sender_in` and `header_match` — so the YAML
merges straight into a real `berger.yaml`.

It also **consolidates** the rules. Several dimensions can point at the
same sender — a domain that is both a top domain and a notification
service, for example. Rather than emit overlapping rules that would tag a
message two or three times, the suggester keeps one rule per sender,
choosing the most specific dimension (newsletter, then notification, then
known contact, then plain domain). The result is a rule set where a
typical message is tagged once — at most twice, when a `header_match` rule
(spam, a mailing list) also applies.

## Persisting a scan

By default the scan writes only the files you asked for and touches no
database. With `--save-report` it also stores the run — the report and the
suggestions, as JSON — in the sidecar's `scan_reports` table, so successive
scans can be compared over time.

## Guarantees

The scan is read-only by construction (PRD v1.1 §4.4):

- **No IMAP action.** The scan reaches the mailbox only through a
  read-only interface that exposes no mutating operation. It cannot move,
  copy, flag, or delete a message.
- **No LLM call.** The scan holds no LLM client; classification is not
  part of the scan.
- **No message body.** The scan reads envelopes and technical headers
  only. Message bodies are never downloaded into the analysis.

## The bidirectional dimension and Sent folders

Dimension 2 — bidirectional contacts — compares who writes to you against
who you write to. It can only do that if Bichon indexes a **Sent** folder
for the account. If no Sent mail is found in the window, the scan logs a
warning and skips this one dimension; every other dimension still runs.
See [`docs/bichon-setup.md`](bichon-setup.md) for which folders Bichon
should index.
