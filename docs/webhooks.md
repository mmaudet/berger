# Webhooks

Webhooks are Berger's gateway to everything beyond plain triage: drafting
replies, push notifications, delegation, task creation. Berger does the
filing; a webhook consumer — n8n, Hermes, LinaTwin, an MCP server, anything
that accepts an HTTP POST — does the rest.

Berger emits one structured event, `berger.tag_applied`. The consumer reads
the `tags` array and decides what to do.

## Contents

- [How emission works](#how-emission-works)
- [Configuration](#configuration)
- [The canonical payload](#the-canonical-payload)
- [Custom payloads with Handlebars](#custom-payloads-with-handlebars)
- [Delivery, retries and auditing](#delivery-retries-and-auditing)
- [Three canonical use-cases](#three-canonical-use-cases)
  - [1. Draft generation](#1-draft-generation)
  - [2. Delegation forward](#2-delegation-forward)
  - [3. Urgent push notification](#3-urgent-push-notification)
- [Example n8n workflows](#example-n8n-workflows)

## How emission works

A webhook fires through a tag's `webhook:` action. The flow, for one
message:

1. Berger computes the message's tags (native filters, then the LLM).
2. For each tag, it looks up the `actions` block. A `webhook: <name>` field
   schedules webhook `<name>`.
3. The distinct webhook names are collected (a webhook referenced by two
   tags fires once).
4. Berger builds the canonical payload once and POSTs it to each webhook,
   honouring each webhook's own `when:` filter.
5. Every emission — success or failure — is recorded in the
   `webhook_emissions` table of the sidecar.

Emission happens *after* the message's IMAP actions have run and the message
has been recorded as processed. A webhook failure can never undo or block
the triage of a message whose foldering already succeeded.

## Configuration

Webhooks are declared in the `webhooks:` section of `berger.yaml` and
referenced by name from `actions`. See [`yaml.md`](yaml.md#webhooks) for the
field-by-field reference. A minimal pair:

```yaml
actions:
  needs-reply:
    copy_to: "a-repondre"
    webhook: linatwin-draft        # ← references the webhook below

webhooks:
  - name: linatwin-draft
    url: "https://n8n.example.com/webhook/linatwin/draft"
    headers:
      Authorization: "Bearer ${HERMES_TOKEN}"
```

A webhook's `when:` list narrows which messages actually fire it; its
`template:` field replaces the canonical body with a rendered one. Both are
optional.

## The canonical payload

Unless a webhook declares a `template`, Berger POSTs this JSON, with
`Content-Type: application/json`. The field order is fixed.

```json
{
  "event": "berger.tag_applied",
  "berger_version": "0.1.0",
  "timestamp": "2026-05-19T08:32:15Z",
  "account": "michel-marie@linagora.com",
  "tags": ["needs-reply", "cat/work"],
  "filters_matched": ["needs-reply", "cat/work"],
  "message": {
    "id": "<abc-def@interieur.gouv.fr>",
    "thread_id": "thread-xyz",
    "from": {
      "name": "Arnaud Clair",
      "email": "arnaud.clair@interieur.gouv.fr"
    },
    "to": [
      {"name": "Michel-Marie Maudet", "email": "michel-marie@linagora.com"}
    ],
    "cc": [],
    "subject": "Validation architecture Zero Trust RAG",
    "date": "2026-05-19T08:28:00Z",
    "body_text": "Bonjour Michel-Marie, ...",
    "body_html": "<p>Bonjour Michel-Marie, ...</p>",
    "has_attachments": false
  },
  "classification": {
    "category": "work",
    "needs_reply": true,
    "priority": 5
  },
  "bichon_message_uri": "https://bichon.example.com/api/v1/messages/abc-def"
}
```

### Field reference

| Field | Type | Description |
|---|---|---|
| `event` | string | Always `berger.tag_applied`. |
| `berger_version` | string | The Berger version that emitted the event. |
| `timestamp` | string | Emission time, RFC 3339 UTC, second precision. |
| `account` | string | The account the message belongs to (its email). |
| `tags` | string[] | Every tag applied to the message. |
| `filters_matched` | string[] | Human-readable identifiers of what fired the tags. |
| `message` | object | The message itself — see below. |
| `classification` | object \| null | The LLM classification, or `null` when no LLM ran. |
| `bichon_message_uri` | string | A link back to the message's copy in Bichon. |

`message`:

| Field | Type | Description |
|---|---|---|
| `id` | string | RFC 822 `Message-ID`. |
| `thread_id` | string | The conversation thread identifier. |
| `from` | object | `{ "name": …, "email": … }`. `name` is empty when the header carried only an address. |
| `to` | object[] | `To:` recipients, each `{ "name": …, "email": … }`. |
| `cc` | object[] | `Cc:` recipients, same shape. |
| `subject` | string | The `Subject:` header. |
| `date` | string | The `Date:` header, RFC 3339 UTC. |
| `body_text` | string | The plain-text body. Empty when the message has no text part. |
| `body_html` | string | The HTML body. Empty when the message has no HTML part. |
| `has_attachments` | bool | Whether the message carries attachments. |

`classification` — present only when the `llm` section is configured and the
classifier succeeded; `null` otherwise:

| Field | Type | Description |
|---|---|---|
| `category` | string | The category label. |
| `needs_reply` | bool | Whether the message expects a personal reply. |
| `priority` | integer | Urgency, 1–5. |

The consumer should treat `classification` as nullable and key its routing
on `tags` (which always reflect the classification when one ran).

## Custom payloads with Handlebars

A webhook may declare a `template` — a [Handlebars](https://handlebarsjs.com)
template rendered against the canonical payload. Berger POSTs the rendered
string verbatim instead of the canonical JSON.

```yaml
webhooks:
  - name: hermes-forward-christelle
    url: "https://n8n.example.com/webhook/berger/delegate"
    template: |
      {"subject": "{{message.subject}}",
       "from": "{{message.from.name}} <{{message.from.email}}>",
       "link": "{{bichon_message_uri}}"}
```

The template sees the whole canonical structure — `{{message.subject}}`,
`{{message.from.email}}`, `{{classification.priority}}`,
`{{bichon_message_uri}}`, and so on. If you emit JSON, you are responsible
for escaping; for anything non-trivial, prefer the canonical payload and
shape it in the consumer.

A template that fails to render is logged as an error for that emission and
does not affect the message's triage.

## Delivery, retries and auditing

Berger's delivery contract (PRD §5.6):

- **Fire-and-forget.** Berger does not wait on the consumer's business
  logic; it only waits for the HTTP response.
- **Bounded retry.** Up to `retry.max_attempts` tries (default 3). With
  `backoff: exponential` the waits are 1s, 4s, 16s; with `backoff: fixed`,
  1s each.
- **What is retried.** Server errors (5xx), `429 Too Many Requests`, and
  transport failures (connection refused, timeout, DNS) are transient and
  retried. Other 4xx responses are permanent — Berger stops at once.
- **No persistent queue.** If Berger crashes between emission and success,
  the event is lost. This is best-effort, not transactional. The message's
  tags are already applied and recorded regardless.
- **Auditing.** Every emission is written to `webhook_emissions` —
  the rendered payload, the HTTP status, the attempt count, the success
  flag. `berger explain <message-id>` and the WebUI `/explain/<id>` page
  replay it.

A consumer should therefore be **idempotent** where it can: a retried POST
may arrive after the consumer already processed the first one (if the first
response was lost). Key any deduplication on `message.id`.

## Three canonical use-cases

These are the three workflows Berger is designed around. Each is a contract
between Berger and the consumer.

### 1. Draft generation

**Goal:** a reply draft appears in the user's `Drafts` folder, ready to
review and send.

```yaml
actions:
  needs-reply:
    copy_to: "a-repondre"
    mark_flagged: true
    webhook: linatwin-draft

webhooks:
  - name: linatwin-draft
    url: "https://n8n.example.com/webhook/linatwin/draft"
    headers:
      Authorization: "Bearer ${HERMES_TOKEN}"
```

Flow:

1. The LLM classifies a message with `needs_reply: true`; Berger applies the
   `needs-reply` tag and emits `linatwin-draft` with the canonical payload.
2. n8n receives it, calls a reply generator (LinaTwin / Mistral / Claude)
   with `message.body_text`, the subject, and — fetched via
   `bichon_message_uri` — the thread history.
3. The generator writes a reply in the user's language and style.
4. n8n performs an **IMAP `APPEND` into the `Drafts` folder** of the source
   account, with the generated reply.
5. The user sees a draft in `Brouillons` / `Drafts`, ready to edit and send.

Berger never generates or sends mail itself — it only signals that a draft
is wanted (PRD §6).

### 2. Delegation forward

**Goal:** a colleague is kept informed of mail tagged for them, without the
user forwarding anything by hand.

```yaml
actions:
  delegate/christelle:
    copy_to: "delegate/christelle"
    webhook: hermes-forward-christelle

webhooks:
  - name: hermes-forward-christelle
    url: "https://n8n.example.com/webhook/berger/delegate"
```

Flow:

1. A tag `delegate/christelle` is applied (by a native `sender_in` rule, or
   an LLM category mapped to that tag).
2. Berger emits `hermes-forward-christelle` with the canonical payload.
3. n8n runs a delegation workflow — for example:
   - a Telegram message to Christelle with the `bichon_message_uri` link, or
   - an entry in a daily 18:00 digest email.
4. Christelle stays informed; the user forwards nothing manually.

### 3. Urgent push notification

**Goal:** a short push notification on a phone or watch for genuinely urgent
mail.

```yaml
actions:
  priority-high:
    mark_flagged: true
    webhook: hermes-push-urgent

webhooks:
  - name: hermes-push-urgent
    url: "https://hermes.example.com/webhook/push/urgent"
    when:
      - priority-high
      - cat/urgent
```

Flow:

1. The LLM classifies a message with `priority` 4 or 5; Berger applies
   `priority-high` and emits `hermes-push-urgent`.
2. The `when:` filter ensures the webhook only ever fires for urgent tags,
   even if a future config references it elsewhere.
3. Hermes routes the event to the configured channels — a personal Telegram
   bot, an Apple Watch or Garmin device via their APIs.
4. The user gets a short notification: sender, subject, and a one-line
   summary if the consumer produced one.

## Example n8n workflows

Berger only needs an HTTP endpoint; n8n's **Webhook** node provides one. The
sketches below outline the node chain — adapt them to your n8n instance.

### Draft generation (use-case 1)

```
Webhook (POST /webhook/linatwin/draft)
  → IF  {{$json.classification.priority}} >= 3      # skip low-value mail
      → HTTP Request   call the reply generator with $json.message.body_text
          → Code       wrap the reply as an RFC 822 message
              → IMAP   APPEND into the "Drafts" folder of the source account
  → (else) NoOp
```

The Webhook node's "Authentication → Header Auth" should match the
`Authorization` header set in the `linatwin-draft` webhook config.

### Delegation digest (use-case 2)

```
Webhook (POST /webhook/berger/delegate)
  → Set            keep {{$json.message.subject}}, .from.email, .bichon_message_uri
      → NoCodeDB / Google Sheets append   queue the item for the digest
# A separate scheduled workflow:
Schedule Trigger (daily 18:00)
  → read the queued items
      → Code   render a digest
          → Send Email   to the colleague   →   clear the queue
```

### Urgent push (use-case 3)

```
Webhook (POST /webhook/push/urgent)
  → Telegram   sendMessage
      chat_id: <your chat id>
      text: "⚠ {{$json.message.from.name}} — {{$json.message.subject}}\n{{$json.bichon_message_uri}}"
```

Because Berger may retry a delivery, make the consumer side idempotent where
it matters — for instance, deduplicate on `{{$json.message.id}}` before
appending a draft, so a retried POST does not create a second one.
