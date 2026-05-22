# Setting up Bichon for Berger

Berger does not read IMAP directly. Every message it triages comes from
[Bichon](https://github.com/mmaudet/bichon), an archiver that connects to
your mail servers, indexes everything, and exposes a REST API. Berger polls
that API, triages, and writes its actions back over IMAP.

So before Berger can do anything useful, Bichon must be running, archiving
the accounts you care about, and reachable from Berger.

## Contents

- [The division of labour](#the-division-of-labour)
- [What Berger needs from Bichon](#what-berger-needs-from-bichon)
- [Excluding Berger's own folders](#excluding-bergers-own-folders) — important
- [Finding your `bichon_account_id`](#finding-your-bichon_account_id)
- [Wiring Bichon into `berger.yaml`](#wiring-bichon-into-bergeryaml)
- [Eventual consistency](#eventual-consistency)
- [Checklist](#checklist)

## The division of labour

```
  Mail servers ──IMAP read──▶ Bichon ──REST──▶ Berger ──IMAP write──▶ source server
```

- **Bichon** reads. It connects to each mail server over IMAP, archives
  messages, builds a full-text index, and never writes back. It is designed
  for static, read-only sources.
- **Berger** writes. It reads messages *from Bichon's API*, decides on tags,
  and then connects to the same source servers over IMAP to copy, move and
  flag messages into `Berger/<tag>/` folders.

Berger breaks Bichon's "the source never changes" assumption — Berger *does*
change the source. The rest of this document is mostly about keeping that
from causing trouble.

## What Berger needs from Bichon

For Berger to work against a Bichon instance you need:

1. **A reachable Bichon base URL** — e.g. `https://bichon.example.com`.
   Berger reaches it over HTTP(S).
2. **An API token** for Bichon — Berger sends it as an HTTP `Bearer`
   credential on every request.
3. **At least one account configured and archiving in Bichon** — Bichon must
   already be polling the mail server and have messages indexed. Berger only
   ever sees what Bichon has archived.

Berger uses three Bichon REST endpoints, for reference:

| Endpoint | Purpose |
|---|---|
| `GET /api/v1/minimal-account-list` | Lists accounts and their numeric ids. |
| `POST /api/v1/search-messages` | Pages through message envelopes since a date. |
| `GET /api/v1/download-message/{account_id}/{envelope_id}` | Fetches the raw RFC 822 message. |

You do not call these yourself — Berger does — but they explain what
permissions the API token must allow: listing accounts, searching, and
downloading message bodies.

## Excluding Berger's own folders

This is the single most important part of the Bichon setup.

Berger writes its triage results into `Berger/*` folders **on the source
IMAP server**. Bichon, archiving that same server, will see those folders on
its next sync — and the same messages would come back to Berger as "new".

Berger already defends against this on its own side, unconditionally:

- Any message whose source folder starts with `Berger/` (or
  `INBOX.Berger.` / `INBOX/Berger/`) is ignored on read.
- Every message is checked against the `processed_messages` ledger by
  `Message-ID` and skipped if already triaged.

So the loop is broken regardless of how Bichon is configured. **But you
should still exclude `Berger/*` in Bichon**, because otherwise Bichon wastes
disk and index space (Tantivy) archiving Berger's own copies — copies that
Berger will never look at.

In Bichon's account configuration, add `Berger/*` (and typically `Junk` and
`Trash`) to the excluded folders:

```toml
[accounts.linagora]
imap_server = "imap.linagora.com"
# … credentials …
excluded_folders = ["Berger/*", "Junk", "Trash"]
```

Adjust the exact key name and syntax to your Bichon version — the principle
is what matters: **tell Bichon not to archive `Berger/*`.**

This changes nothing functionally for Berger (its own read-side filter
already protects it). It is purely a resource optimisation — but a worthwhile
one, since Berger's folders grow with every triaged message.

## Finding your `bichon_account_id`

Each account in `berger.yaml` needs a `bichon_account_id` — the **numeric**
id Bichon assigns the account. It is not the email address and not the
account label.

Query Bichon's account list with the same token Berger will use:

```sh
curl -s -H "Authorization: Bearer $BICHON_API_TOKEN" \
  https://bichon.example.com/api/v1/minimal-account-list
```

The response is a JSON array; each entry has an `id` and an `email`:

```json
[
  {"id": 8525922389589073, "email": "you@linagora.com"},
  {"id": 1417038252461348, "email": "you@gmail.com"}
]
```

Put each `id` into `.env` as `BICHON_ACCOUNT_ID_1`, `BICHON_ACCOUNT_ID_2`,
… — one per account. It must be numeric; Berger parses it back to an integer
for the search API.

## Wiring Bichon into `berger.yaml`

With the URL, token and account ids in hand, the `bichon` and `accounts`
sections look like this:

```yaml
bichon:
  base_url: "${BICHON_BASE_URL}"
  api_token: "${BICHON_API_TOKEN}"      # never inline the token

accounts:
  - name: "account-1"
    bichon_account_id: "${BICHON_ACCOUNT_ID_1}"
    imap:
      host: "${IMAP_HOST_1}"
      user: "${IMAP_USER_1}"
      password: "${IMAP_PASSWORD_1}"
```

Note the two distinct credential paths:

- `bichon.api_token` lets Berger **read** from Bichon.
- `accounts[].imap` lets Berger **write** back to the source server.

They are independent — Bichon's token has nothing to do with the IMAP
password. See [`yaml.md`](yaml.md) for the full field reference and
[`ops.md`](ops.md) for how to supply the `${VAR}` secrets.

You can confirm Berger reaches Bichon without applying anything by running a
dry run — it polls Bichon but performs no IMAP action:

```sh
berger dry-run --config berger.yaml
```

## Eventual consistency

Berger and Bichon are not synchronised. When Berger moves a message:

```
T+0s    Berger COPYs the message to Berger/junk/, then removes it from INBOX
T+0s    the message is gone from INBOX, present in Berger/junk/
T+0..N  Bichon does not know yet — its index still says "INBOX"
T+N     Bichon runs its next incremental sync and updates the index
```

`N` is Bichon's poll interval — typically 30 seconds to 5 minutes. For that
window, Bichon's index and the real IMAP state disagree. This is expected
and harmless: Berger's idempotency ledger, not Bichon's index, decides what
has been triaged.

Plan for one to five minutes of lag between a Berger action and Bichon's
view of it.

## Checklist

Before pointing Berger at a real account, confirm:

- [ ] Bichon is running and reachable at the `base_url` you will configure.
- [ ] You have an API token that can list accounts, search, and download
      messages.
- [ ] The account is configured in Bichon and has finished an initial
      archive (Berger only sees archived mail).
- [ ] You know the account's numeric `bichon_account_id`.
- [ ] `Berger/*` is in Bichon's `excluded_folders` for that account
      (recommended — saves Bichon disk and index space).
- [ ] `berger dry-run` against your `berger.yaml` returns without error.
