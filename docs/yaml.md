# `berger.yaml` — configuration reference

Berger is configured by a single YAML file, loaded once at startup. There is
no hot reload: a configuration change takes effect on the next restart.

This document describes every section and every field. For a complete,
working example, see [`berger.example.yaml`](../berger.example.yaml) at the
repository root.

## Contents

- [Top-level structure](#top-level-structure)
- [Environment variable interpolation](#environment-variable-interpolation)
- [`bichon`](#bichon)
- [`database`](#database)
- [`accounts`](#accounts)
- [`filters`](#filters) — the four native filter types
- [`llm`](#llm)
- [`actions`](#actions) — the five action primitives
- [`webhooks`](#webhooks)
- [Tags: where they come from](#tags-where-they-come-from)
- [Validation](#validation)

## Top-level structure

```yaml
bichon:    { … }   # required — the upstream archiver
database:  { … }   # required — the SQLite sidecar
accounts:  [ … ]   # required — at least one mail account
filters:   [ … ]   # optional — native filter rules
llm:       { … }   # optional — the LLM classifier
actions:   { … }   # optional — per-tag IMAP actions
webhooks:  [ … ]   # optional — named webhook endpoints
```

`bichon`, `database` and `accounts` are mandatory. The other four sections
default to empty when omitted — Berger then simply does less (no native
filters, no LLM, no IMAP writeback, no webhooks).

## Environment variable interpolation

Anywhere in the file, `${NAME}` is replaced with the value of the
environment variable `NAME` before the YAML is parsed. This is how secrets
stay out of the file.

```yaml
bichon:
  api_token: "${BICHON_API_TOKEN}"
```

Rules:

- An **unset** variable is a fatal startup error. Berger never substitutes
  an empty string.
- An unterminated `${` or an empty `${}` is a fatal error.
- Substitution is plain textual replacement — it happens before YAML
  parsing, so a variable may hold any value.
- Because it is textual and runs before parsing, a `${...}` is replaced
  **even inside a `#` comment** — never write a literal `${...}` in a
  comment unless you mean it.

Use `${VAR}` for everything that varies between deployments — the Bichon
URL, account hosts and ids, every credential. Copy `.env.example` to `.env`
and fill it in. With Docker, docker compose loads `.env` automatically;
under systemd, point an `EnvironmentFile` at it (see [`ops.md`](ops.md)).

## `bichon`

How to reach the upstream Bichon instance. Berger reads **all** mail through
Bichon's REST API; it never connects to IMAP for reading.

```yaml
bichon:
  base_url: "${BICHON_BASE_URL}"
  api_token: "${BICHON_API_TOKEN}"
```

| Field | Type | Required | Description |
|---|---|---|---|
| `base_url` | string | yes | Base URL of the Bichon instance. A trailing slash is trimmed automatically. |
| `api_token` | string (secret) | yes | Bearer token sent on every Bichon request. |

See [`bichon-setup.md`](bichon-setup.md) for configuring Bichon itself.

## `database`

The SQLite sidecar — Berger's single source of truth. It is created on
first run and opened in WAL mode.

```yaml
database:
  path: "berger.db"
```

| Field | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Path to the sidecar file. Relative paths resolve against the process working directory. |

The sidecar holds seven tables — processed messages, applied tags, filter
matches, LLM decisions, executed actions, webhook emissions, accounts. It is
never purged automatically; back it up with the recipe in [`ops.md`](ops.md).

## `accounts`

The mail accounts to triage. At least one is required; Berger handles
several.

```yaml
accounts:
  - name: "account-1"
    bichon_account_id: "${BICHON_ACCOUNT_ID_1}"
    imap:
      host: "${IMAP_HOST_1}"
      port: 993
      user: "${IMAP_USER_1}"
      password: "${IMAP_PASSWORD_1}"
```

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | A label for the account. Must be unique across all accounts. |
| `bichon_account_id` | string | yes | The numeric id Bichon assigns the account. It is a string in the YAML but must parse as a number. |
| `imap` | object | yes | The IMAP server, used for **writeback only**. |

### `accounts[].imap`

Berger connects here over IMAPS to apply triage actions (copy, move, flag).
It is never used for reading mail — that is Bichon's job.

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `host` | string | yes | — | IMAP server hostname. |
| `port` | integer | no | `993` | IMAPS port. |
| `user` | string | yes | — | IMAP login. |
| `password` | string (secret) | yes | — | IMAP password. |

Only IMAPS (implicit TLS) is supported; there is no STARTTLS or plaintext
option. Unknown fields under `imap` are rejected.

## `filters`

Native filters are deterministic, no-LLM rules (PRD §5.2). The `filters`
section is a list; **each entry declares exactly one of the four filter
types** plus the `tag` it emits when it matches.

```yaml
filters:
  - sender_in: ["notifications@github.com", "github.com"]
    tag: notif/github
  - subject_regex: "(?i)facture"
    tag: cat/finance
  - list_unsubscribe: true
    tag: bulk
  - header_match:
      header: "X-Spam-Flag"
      pattern: "(?i)yes"
    tag: spam-confirme
```

Every rule entry has:

| Field | Type | Required | Description |
|---|---|---|---|
| `tag` | string | yes | The tag applied to a message this rule matches. |
| *one filter type* | — | yes | Exactly one of `sender_in`, `subject_regex`, `list_unsubscribe`, `header_match`. |

A rule with zero filter types, or with two, is a fatal validation error. An
unknown field in a rule is rejected.

There is **no precedence between rules**: a message keeps every tag it
matches and can therefore land in several folders. The broad
"newsletter vs marketing vs real mail" judgement is the LLM's job, not a
native rule.

### `sender_in`

Matches when the message's sender is in a list of addresses and/or domains.

```yaml
- sender_in: ["notifications@github.com", "github.com", "gitlab.com"]
  tag: notif/forge
```

- Each list entry is either a **full address** (`notifications@github.com`)
  or a **bare domain** (`github.com`).
- Matching is case-insensitive. The sender's address is extracted from the
  `From` header — `GitHub <Notifications@GitHub.com>` matches both
  `notifications@github.com` and `github.com`.
- A bare domain matches any address at that domain.
- The list must be non-empty.

### `subject_regex`

Matches when the `Subject` header matches a regular expression.

```yaml
- subject_regex: "(?i)\\b(facture|invoice|reçu)\\b"
  tag: cat/finance
```

- The value is a [Rust `regex`](https://docs.rs/regex) pattern. It is a
  partial match — the pattern need not match the whole subject.
- Prefix with `(?i)` for case-insensitivity.
- Remember YAML string escaping: a backslash in a double-quoted scalar must
  be doubled (`\\b`), or use a single-quoted scalar.
- An invalid regex is a fatal startup error.

### `list_unsubscribe`

Matches when the message carries a `List-Unsubscribe` header (RFC 2369) —
the reliable marker of bulk and automated mail.

```yaml
- list_unsubscribe: true
  tag: bulk
```

- The value must be `true`. (`false` declares no filter type and is a
  validation error.)
- Only the header's *presence* is checked; its value is ignored.

### `header_match`

Matches when a named header's value matches a regular expression.

```yaml
- header_match:
    header: "Auto-Submitted"
    pattern: "(?i)auto-(generated|replied)"
  tag: automated
```

| Sub-field | Type | Required | Description |
|---|---|---|---|
| `header` | string | yes | The header name to inspect. Matching of the name is case-insensitive. |
| `pattern` | string | yes | A Rust regex matched (partially) against the header's value. |

Useful for `X-Spam-Flag`, `Auto-Submitted`, `Precedence: bulk`,
`X-GitHub-Reason`, and similar. An invalid regex is a fatal startup error.

## `llm`

The LLM classifier (PRD §5.3) — an OpenAI-compatible chat-completions
endpoint. The whole section is **optional**: omit it to triage on native
filters alone.

```yaml
llm:
  endpoint: "${LLM_ENDPOINT}"
  model: "${LLM_MODEL}"
  api_key: "${LLM_API_KEY}"
  categories:
    - work
    - perso
    - newsletter
    - finance
```

| Field | Type | Required | Description |
|---|---|---|---|
| `endpoint` | string | yes | Full URL of the chat-completions endpoint. |
| `model` | string | yes | The model name, e.g. `mistral-small-latest` or `gemma3:12b`. |
| `api_key` | string (secret) | no | Bearer credential. Omit for a local endpoint (e.g. Ollama) that needs no auth. |
| `categories` | list of strings | no | The category vocabulary offered to the model. |

Berger talks the OpenAI chat-completions protocol, so any compatible
endpoint works without code changes — the Mistral API and a local Ollama
server are both validated.

**The classifier returns three fields** for each message:

- `category` — a short label.
- `needs_reply` — whether the message expects a personal reply.
- `priority` — urgency, 1 (trivial) to 5 (urgent).

**`categories`** constrains `category`. When the list is set, the model is
asked to return *exactly one* of those labels; when omitted, it invents a
lowercase label of its own. A fixed vocabulary keeps your `cat/*` folders
stable and predictable — recommended for production.

Every LLM call is cached on `(Message-ID, hash(prompt))`: a message is
classified at most once for a given prompt. If a call fails or the model
returns output that is not valid classification JSON, the message is tagged
`llm_error` and triage continues — the LLM never blocks the pipeline.

Unknown fields under `llm` are rejected.

## `actions`

The `actions` section maps a **tag** to the IMAP operations Berger performs
for it (PRD §5.5). It is a mapping — tag name to an action block.

```yaml
actions:
  notif/github:
    copy_to: "notifs/github"
    mark_seen: true
  bulk:
    move_to: "bulk"
    mark_seen: true
  needs-reply:
    copy_to: "a-repondre"
    mark_flagged: true
    webhook: linatwin-draft
```

A tag with **no entry** in `actions` is still recorded in the sidecar, but
the message is left untouched on the server — triage is non-destructive by
default.

Each action block has five optional fields — the five primitives:

| Primitive | Type | IMAP effect | Reversible |
|---|---|---|---|
| `copy_to` | string | `COPY` the message into `Berger/<folder>` | yes — the original stays in INBOX |
| `move_to` | string | `COPY` into `Berger/<folder>`, then remove from INBOX | hard — the message leaves INBOX |
| `mark_seen` | bool | set the `\Seen` flag | yes |
| `mark_flagged` | bool | set the `\Flagged` flag (the star/flag in clients) | yes |
| `webhook` | string | POST the canonical event to the named webhook | n/a |

Notes:

- **Folder paths.** `copy_to` / `move_to` take a logical path *below*
  `Berger/`, `/`-separated. `copy_to: notifs/github` writes to
  `Berger/notifs/github`. Berger prefixes `Berger/` itself.
- **Folders are created on demand.** Before every copy or move, Berger
  checks the destination exists; if a user has deleted it, Berger recreates
  and subscribes it, logging a `WARN`.
- **`copy_to` is the safe default.** `move_to` removes the message from the
  INBOX — choose it explicitly, tag by tag, and only for mail you are happy
  to file away (bulk, notifications, confirmed spam).
- **No deletion.** There is no `delete` primitive. The only operation that
  removes a message from a folder is `move_to`, and it removes only that one
  message — never an `EXPUNGE` of anything else.
- **`webhook`** names an entry in the [`webhooks`](#webhooks) section.
- **Execution order is not significant.** Within a message, flags and
  copies run first (while the message is still in INBOX), the move runs
  last. If several tags target the same folder with both `copy_to` and
  `move_to`, `move_to` wins (logged as a `WARN`). A message can move only
  once; extra `move_to` targets are ignored with a `WARN`.

Unknown fields in an action block are rejected.

## `webhooks`

Named HTTP endpoints that a tag's `webhook:` action POSTs to (PRD §5.6).
The `webhooks` section is a list.

```yaml
webhooks:
  - name: linatwin-draft
    url: "${WEBHOOK_DRAFT_URL}"
    method: POST
    headers:
      Authorization: "Bearer ${WEBHOOK_DRAFT_TOKEN}"
    retry:
      max_attempts: 3
      backoff: exponential
    when:
      - needs-reply
    template: null
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | string | yes | — | The webhook's name — referenced by a tag's `webhook:` action. Unique across webhooks. |
| `url` | string | yes | — | The endpoint to POST to. |
| `method` | string | no | `POST` | The HTTP method. Only `POST` is meaningful at v1. |
| `headers` | map | no | `{}` | Extra HTTP headers sent with every request — e.g. `Authorization`. |
| `retry` | object | no | see below | The retry policy. |
| `when` | list of strings | no | `[]` | Tag filter — see below. |
| `template` | string | no | none | A Handlebars template for the request body — see below. |

### `retry`

| Sub-field | Type | Default | Description |
|---|---|---|---|
| `max_attempts` | integer | `3` | Total attempts, including the first. |
| `backoff` | enum | `exponential` | `exponential` (1s, 4s, 16s …) or `fixed` (1s between every attempt). |

Delivery is fire-and-forget: a webhook that exhausts its budget is logged
and audited in the sidecar, never propagated as an error. Server errors
(5xx) and `429 Too Many Requests` are retried; other 4xx responses are
permanent and stop the retry loop immediately.

### `when`

When `when` is non-empty, the webhook fires only if the message carries at
least one of the listed tags — even if another tag also references the
webhook. An empty `when` (the default) places no restriction.

```yaml
when:
  - priority-high
  - cat/urgent
```

### `template`

When omitted, the webhook receives the **canonical JSON payload**
(`berger.tag_applied`) — see [`webhooks.md`](webhooks.md). When set,
Berger renders the [Handlebars](https://handlebarsjs.com) template against
that same payload and POSTs the rendered string instead.

```yaml
template: |
  {"subject": "{{message.subject}}", "from": "{{message.from.email}}"}
```

The template has access to the entire canonical payload structure. A broken
template is reported as an error for that emission; it does not stop triage.

Unknown fields in a webhook entry are rejected.

## Tags: where they come from

`actions` keys and webhook `when` lists reference *tags*. Tags come from two
places:

1. **Native filters** emit whatever `tag` you write in the `filters` rule —
   you choose these names freely (`notif/github`, `bulk`, `spam-confirme`).

2. **The LLM classifier** emits a fixed set derived from the classification
   (PRD §5.4):
   - `cat/<category>` — always, where `<category>` is the model's
     `category` (so a `categories:` list of `work`, `perso`, … yields
     `cat/work`, `cat/perso`, …).
   - `needs-reply` — when the classification's `needs_reply` is true.
   - `priority-high` — when the classification's `priority` is 4 or 5.
   - `llm_error` — instead of the above, when the classifier fails.

To act on an LLM result, add `actions` entries for the `cat/*` labels you
expect plus `needs-reply`, `priority-high` and `llm_error`. Tags with no
`actions` entry are recorded but cause no IMAP action.

## Validation

Berger validates the whole configuration at startup and refuses to run on
any error. The checks include:

- `bichon.base_url`, `bichon.api_token` and `database.path` are non-empty.
- At least one account; account names are unique; each account's
  `name`, `bichon_account_id`, `imap.host`, `imap.user` and `imap.password`
  are non-empty.
- Every filter rule declares **exactly one** filter type and a non-empty
  `tag`; every `subject_regex` / `header_match` pattern is a valid regex.
- `llm.endpoint` and `llm.model` are non-empty when the `llm` section is
  present.
- Webhook names are unique; each webhook has a non-empty `name` and `url`.
- Unknown fields under `imap`, a filter rule, an action block, the `llm`
  section or a webhook entry are rejected.

A `${VAR}` referencing an unset environment variable is a fatal error
before validation even begins.
