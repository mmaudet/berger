# PRD — Berger v1.1 · Scan d'inbox initial

| | |
|---|---|
| **Projet** | Berger |
| **Type de document** | PRD additif (overlay v1.0) |
| **Version produit** | 1.1 (release `v0.2.0`) |
| **Révision document** | 1 — 22 mai 2026 |
| **Statut** | Validé, à implémenter **après** release `v0.1.0` |
| **Auteur** | Michel-Marie Maudet |
| **Licence** | AGPLv3 |
| **Durée cible** | 3 jours de sprint dense |
| **Prérequis** | Release `v0.1.0` (PRD v1.0) terminée, taguée, en production |

---

## 1. Préambule

Ce document **ne modifie pas** la v1.0 (cf. `docs/PRD.md`). Il décrit une fonctionnalité additive à implémenter dans une session distincte, après que la `v0.1.0` ait été taguée et que le démon ait tourné en production sur le compte de l'auteur pendant au moins 7 jours.

**Règles d'application :**

- Ce PRD ne réécrit aucune ligne de code existante. Il ajoute un module Rust orthogonal (`src/scan/`), une nouvelle commande CLI (`berger scan`), une nouvelle route WebUI optionnelle, et au plus une nouvelle table SQLite (`scan_reports`).
- Le pipeline de tri existant (ingest → filters → llm → tags → actions → webhooks) n'est pas touché.
- Les filtres, les actions IMAP, les webhooks, le sidecar SQLite hors `scan_reports`, la config YAML existante : **intouchables**.
- Si une contradiction apparaît entre ce PRD et le PRD v1.0, le PRD v1.0 prévaut. L'addendum s'adapte, pas l'inverse.

**Position dans la roadmap :**

Ce PRD couvre **uniquement le scan d'inbox**. Les autres items v1.1 mentionnés dans le PRD v1.0 (backfill historique, JMAP keywords writeback, reload config à chaud, métriques Prometheus, queue persistante webhooks, `berger replay-webhook`, `berger reconcile`, etc.) font l'objet de PRDs séparés et seront implémentés indépendamment.

## 2. Pitch

`berger scan` analyse l'inbox réelle de l'utilisateur sur N derniers jours (30 par défaut), repère statistiquement les patterns récurrents (expéditeurs fréquents, services de notification utilisés, newsletters actives, langue dominante, volumes), et produit un fichier YAML de **suggestions de configuration**, prêt à être validé manuellement et fusionné avec `berger.yaml`.

Le scan est **strictement read-only** : aucune action IMAP, aucun appel LLM, aucun écriture sur le serveur source.

## 3. Vision

Aujourd'hui, l'utilisateur qui installe Berger doit éditer un fichier YAML générique (`berger.example.yaml`) en l'adaptant à son contexte. Cet effort de personnalisation est un point de friction majeur : sans connaître les patterns réels de son inbox, l'utilisateur configure à l'aveugle, soit en sur-spécifiant (règles inutiles), soit en sous-spécifiant (gros bruit non capturé).

`berger scan` inverse le paradigme : **Berger observe l'inbox et propose la configuration**, pas l'inverse. L'utilisateur valide, ajuste, et ne part pas de zéro.

C'est l'équivalent moderne du *"premier scan d'inbox"* qu'aucun outil open-source de tri email n'a jamais offert proprement. Fyxer, Cora, Superhuman le font côté propriétaire en envoyant ton inbox à leurs serveurs. Berger le fait localement, gratuitement, en lecture seule.

## 4. Scope MVP de la v1.1

### 4.1 Commande CLI principale

```
berger scan [OPTIONS]
```

**Options :**

- `--since <DURATION>` : période analysée. Défaut `30d`. Accepte `7d`, `30d`, `90d`, `180d`.
- `--account <NAME>` : limiter à un compte. Défaut : tous les comptes configurés.
- `--output <PATH>` : chemin de sortie pour le YAML suggéré. Défaut : `./berger-scan-<timestamp>.yaml`.
- `--format <FORMAT>` : `yaml` (défaut), `text`, `json`, `all`.
- `--min-evidence <N>` : seuil minimum de mails pour suggérer une règle. Défaut 5.
- `--save-report` : persiste le rapport complet dans le sidecar SQLite (table `scan_reports`).

### 4.2 Dimensions analysées

Sur la période sélectionnée, le scan calcule :

| # | Dimension | Source de données | Sortie utile |
|---|---|---|---|
| 1 | **Top 50 expéditeurs par volume** (mails reçus) | Header `From` | Suggestions `sender_in` candidates |
| 2 | **Top 30 expéditeurs bidirectionnels** | Croisement `From` reçus × `To` envoyés (dossier Sent) | Liste VIP, clients actifs, collègues clés |
| 3 | **Domaines récurrents** | Suffixe `@<domain>` du From | Suggestions `sender_in` par domaine (`*@bouygues.com`) |
| 4 | **Newsletters détectées** | Présence de `List-Unsubscribe` | Recense les newsletters réelles par expéditeur, avec volume |
| 5 | **Mailing-lists actives** | Présence de `List-Id` | Suggère des règles `list_id_match` par mailing-list |
| 6 | **Services notification** | `Auto-Submitted`, `Precedence: bulk`, From `noreply@*` | Recense les services notification *réellement utilisés* (GitHub, GitLab, AWS, etc.) |
| 7 | **Patterns spam observés** | `X-Spam-Flag`, `X-Spam-Status`, scores | Calibre l'agressivité des règles spam selon le MTA amont |
| 8 | **Patterns sujet récurrents** | N-grammes (2-3 mots) dans `Subject`, post-stopwords FR/EN | Suggestions `subject_regex` (factures, releases, etc.) |
| 9 | **Langue dominante** | Détection langue par échantillonnage (10% des mails) | Ajuste recommandation `system_prompt` du LLM (biais FR/EN) |
| 10 | **Volume horaire** | Histogramme des `Date` | Recommande un `poll_interval_seconds` adapté |

### 4.3 Sorties

**Sortie 1 — YAML suggéré (format par défaut).** Un fichier YAML valide, parseable par Berger, contenant des règles candidates annotées. Format :

```yaml
# ============================================================================
# Suggestions de configuration auto-générées par berger scan
# Date du scan : 2026-05-22T14:32:15Z
# Période analysée : 30 derniers jours
# Volume analysé : 7847 mails sur 2 comptes
# Seuil de pertinence : 5 mails minimum
# ============================================================================
#
# À fusionner manuellement avec votre berger.yaml. Chaque suggestion est
# annotée par confidence (0-1) et evidence (justification statistique).
# Validez chaque règle avant intégration.
# ============================================================================

scan_metadata:
  scan_id: "scan_2026-05-22T14-32-15Z"
  generated_at: "2026-05-22T14:32:15Z"
  berger_version: "0.2.0"
  analysis_period: "30d"
  total_messages_analyzed: 7847
  accounts: ["linagora-pro", "gmail-perso"]

suggested_filters:

  - name: scan-suggested-bouygues
    type: sender_in
    senders: ["*@bouyguestelecom.fr"]
    tags: ["client/Bouygues"]
    evidence:
      messages_received: 47
      messages_sent_to: 23
      bidirectional_ratio: 0.49
      first_seen: "2026-04-15"
      last_seen: "2026-05-21"
    confidence: 0.93
    suggested_by: "scan_2026-05-22T14-32-15Z"
    rationale: "Échange bidirectionnel soutenu sur 30 jours, ratio de réponse élevé"

  - name: scan-suggested-substack-newsletters
    type: list_id_match
    pattern: "*.substack.com"
    tags: ["newsletter", "newsletter/substack"]
    evidence:
      messages_received: 142
      distinct_senders: 12
      list_unsubscribe_present: true
      avg_per_week: 33
    confidence: 0.99
    suggested_by: "scan_2026-05-22T14-32-15Z"
    rationale: "12 newsletters Substack distinctes, volume élevé, présence systématique de List-Unsubscribe"

# (suite : 50-100 suggestions selon volume de l'inbox)

suggested_settings:
  poll_interval_seconds: 90
  evidence: "Pic horaire 10h-12h, 30 mails/heure max. Polling à 90s suffisant."

suggested_llm_hint:
  dominant_language: "fr"
  evidence: "84% FR, 14% EN, 2% autres (sur échantillon de 800 mails)"
  prompt_recommendation: "Biaiser le system_prompt LLM en français explicite."
```

**Sortie 2 — Rapport texte (`--format text`).** Résumé de 50-100 lignes, lisible en CLI, pour décision rapide.

```
================================================================================
BERGER SCAN REPORT — 2026-05-22T14:32:15Z
================================================================================

Période analysée : 30 derniers jours
Volume total    : 7,847 mails
Comptes scannés : linagora-pro, gmail-perso

--- TOP EXPÉDITEURS BIDIRECTIONNELS (échange actif) ---------------------------

  arnaud.clair@interieur.gouv.fr            47 reçus / 23 envoyés (ratio 0.49)
  pascal.vilarem@linagora.com               89 reçus / 41 envoyés (ratio 0.46)
  christelle.bernard@linagora.com           62 reçus / 38 envoyés (ratio 0.61)
  contact@bouyguestelecom.fr                34 reçus / 18 envoyés (ratio 0.53)
  ...

--- TOP DOMAINES ENTRANTS -----------------------------------------------------

  linagora.com               428 mails  (interne)
  noreply.github.com         284 mails  (notif Cc)
  *.gouv.fr                  178 mails  (5 domaines distincts)
  substack.com               142 mails  (12 newsletters)
  ...

--- NEWSLETTERS DÉTECTÉES (présence de List-Unsubscribe) ----------------------

  847 mails avec List-Unsubscribe  (10.8% du total)
    → Substack       142 mails / 12 expéditeurs
    → Medium          89 mails /  3 expéditeurs
    → Hacker News     45 mails /  1 expéditeur
    ...

--- SERVICES NOTIFICATION DÉTECTÉS --------------------------------------------

  GitHub             284 mails  (mention: 18, review: 12, security: 4, autres: 250)
  GitLab              47 mails
  LinkedIn            68 mails
  AWS                 22 mails
  OVH                 14 mails

--- PATTERNS SPAM OBSERVÉS ----------------------------------------------------

  X-Spam-Flag: YES         12 mails
  X-Spam-Score >= 5.0      31 mails
  Authentication: dmarc=fail   8 mails
  
  Recommandation : MTA amont (Apache James) actif, règles spam pertinentes.

--- LANGUE DOMINANTE ----------------------------------------------------------

  Français   84%
  Anglais    14%
  Autres      2%

--- RECOMMANDATIONS PRINCIPALES -----------------------------------------------

  → 14 règles sender_in suggérées (confidence > 0.8)
  → 6 règles list_id_match suggérées
  → 3 règles subject_regex suggérées
  → poll_interval_seconds: 90 (au lieu du défaut 60)
  → Prompt LLM : biaiser fortement vers le français
  → 142 mails Substack à archiver (move_to: newsletters)

Fichier de suggestions complet : ./berger-scan-2026-05-22T14-32-15Z.yaml
================================================================================
```

**Sortie 3 — JSON (`--format json`).** Sortie machine-readable pour intégration tierce (n8n, dashboards, etc.). Schéma documenté dans `docs/scan.md`.

**Sortie 4 — Rapport WebUI (optionnel, si temps disponible).** Une nouvelle route `/scan` dans la WebUI Axum existante qui liste les scans persistés (si `--save-report`) et permet d'inspecter le rapport. Format identique au texte mais formaté HTML. **Non bloquant pour la release.**

### 4.4 Garde-fous

- **Seuil minimum d'evidence.** Aucune suggestion produite si `messages_received < min_evidence` (défaut 5). Évite les règles bruyantes basées sur un seul mail.
- **Confidence calculée.** Formule simple : `confidence = min(1.0, log(messages) / 4 + bidirectional_ratio * 0.3)`. Documentée dans `docs/scan.md`.
- **Pas d'application automatique, jamais.** Le YAML suggéré n'est jamais fusionné automatiquement. L'utilisateur copie-colle ce qu'il veut. Cette garantie est non-négociable.
- **Read-only strict.** Le code du scan a un type Rust dédié `ReadOnlyMessageSource` qui interdit au compile-time toute écriture IMAP. Implémenté comme un trait sans méthode mutante.
- **Privacy.** Le scan lit `From`, `To`, `Cc`, `Subject`, `Date`, et les headers techniques (`List-*`, `X-Spam-*`, `Auto-Submitted`, `Precedence`, `Authentication-Results`, `Content-Type`). **Le scan ne lit jamais le corps du mail.** À documenter explicitement dans `docs/scan.md` et le `--help`.

## 5. Architecture & implémentation

### 5.1 Nouveau module Rust

Un nouveau module orthogonal `src/scan/`, sans dépendance avec `pipeline/` ou `actions/`. Structure proposée (Claude Code a latitude sur le détail) :

```
src/scan/
├── mod.rs              # API publique du module
├── analyzer.rs         # Agrégation statistique
├── analyzers/
│   ├── senders.rs      # Dimensions 1, 2, 3
│   ├── newsletters.rs  # Dimension 4
│   ├── lists.rs        # Dimension 5
│   ├── notifications.rs # Dimension 6
│   ├── spam.rs         # Dimension 7
│   ├── subjects.rs     # Dimension 8 (n-grammes)
│   ├── language.rs     # Dimension 9 (détection langue)
│   └── volume.rs       # Dimension 10
├── suggester.rs        # Conversion analyses → règles YAML candidates
├── formatter.rs        # Rendu YAML / texte / JSON
└── storage.rs          # Persistance optionnelle (table scan_reports)
```

### 5.2 Réutilisation des composants v1.0

Le scan **réutilise** sans modification :

- Le client Bichon REST (`src/ingest/`) pour lire les mails. Mêmes méthodes que le pipeline normal.
- Le sidecar SQLite, en lecture seule pour les données existantes.
- La WebUI Axum (`src/webui/`) si la route `/scan` est ajoutée.
- La CLI `clap` (`src/cli/`) : ajout d'une nouvelle sous-commande `scan`.

### 5.3 Nouvelle table SQLite (optionnelle)

Si `--save-report` est utilisé, une seule table additionnelle :

```sql
CREATE TABLE IF NOT EXISTS scan_reports (
    scan_id TEXT PRIMARY KEY,
    started_at TIMESTAMP NOT NULL,
    completed_at TIMESTAMP NOT NULL,
    berger_version TEXT NOT NULL,
    period_days INTEGER NOT NULL,
    accounts_json TEXT NOT NULL,
    total_messages_analyzed INTEGER NOT NULL,
    report_yaml TEXT NOT NULL,
    report_json TEXT NOT NULL,
    report_text TEXT NOT NULL
);
```

Pas de migration breaking. Cette table est créée par une nouvelle migration refinery `v002__add_scan_reports.sql` qui s'applique au démarrage.

### 5.4 Détection bidirectionnelle

Pour la dimension 2 (expéditeurs bidirectionnels), Berger doit savoir quels mails ont été envoyés par l'utilisateur. Deux approches possibles :

- **Approche A (recommandée) :** Lire via Bichon le dossier "Sent" / "Brouillons envoyés" / "Éléments envoyés" du compte source. Bichon est censé indexer tous les dossiers (sauf exclusions). Croiser `To` des mails du Sent avec `From` des mails de l'INBOX.

- **Approche B (fallback) :** Pas de Sent disponible côté Bichon. Le scan tombe en mode dégradé sur la dimension 2 (pas de bidirectionnalité), tout le reste fonctionne. Log un WARN.

La doc `docs/scan.md` explique comment configurer Bichon pour exposer le dossier Sent si nécessaire.

### 5.5 Algorithmes

- **Comptage par From / To / domaine :** trivial, HashMap + tri.
- **N-grammes de sujets :** tokenisation simple sur whitespace + ponctuation, stopwords FR + EN (listes embarquées), 2-grammes et 3-grammes, top 30 par fréquence.
- **Détection langue :** crate `whatlang` (single dependency, MIT, sans modèle externe). Sur 10% des mails échantillonnés au hasard pour réduire coût.
- **Confidence :** formule explicite dans `suggester.rs`, testée unitairement, documentée.

## 6. Jalons de livraison — sprint 3 jours

| Jour | Livrable |
|---|---|
| J1 | Module `src/scan/` initialisé, sous-commande CLI `scan`, lecture Bichon en mode read-only, dimensions 1-3 (top senders, bidirectionnels, domaines), tests unitaires |
| J2 | Dimensions 4-7 (newsletters, mailing-lists, notifications, spam), formatter texte + YAML, garde-fous min-evidence et confidence |
| J3 | Dimensions 8-10 (subjects n-grammes, langue, volume), persistance optionnelle (`scan_reports`), formatter JSON, doc `docs/scan.md`, release `v0.2.0` |

## 7. Critères d'acceptation v1.1

- [ ] `berger scan --help` documente toutes les options
- [ ] `berger scan` sans option produit un rapport texte sur stdout + un fichier YAML
- [ ] `berger scan --format yaml --output suggested.yaml` produit un YAML valide
- [ ] Le YAML suggéré est parseable par le loader de config existant (vérifié par test d'intégration)
- [ ] `berger scan --format json` produit un JSON conforme au schéma documenté
- [ ] Test d'intégration sur dataset synthétique : 100 mails fabriqués avec patterns connus, scan retrouve les patterns
- [ ] **Test critique read-only :** test d'intégration qui mesure le nombre de commandes IMAP STORE/COPY/EXPUNGE émises par Berger pendant un scan → doit être strictement 0
- [ ] **Test critique no-LLM :** test d'intégration qui mesure le nombre d'appels au client LLM pendant un scan → doit être strictement 0
- [ ] Le scan ne lit jamais le corps des mails (vérifié par audit du code : aucun accès à `message.body_text` ou `message.body_html` dans `src/scan/`)
- [ ] Seuil `--min-evidence` respecté : aucune suggestion en sortie avec evidence < seuil
- [ ] Documentation `docs/scan.md` complète (usage, format de sortie, formule confidence, garde-fous privacy)
- [ ] Pas de régression sur les critères d'acceptation v1.0

## 8. Risques & mitigations

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| Performance sur grosse inbox (>100k mails) | Moyen | Moyen | Pagination via Bichon, traitement en streaming, pas de tout-en-mémoire. Documenter limite indicative 100k mails / scan. |
| Bichon ne sync pas le dossier Sent → bidirectionnalité absente | Moyen | Faible | Mode dégradé documenté. Le reste du scan fonctionne. Warn dans le rapport. |
| Faux positifs dans les suggestions | Élevé | Faible | Seuil `min_evidence` configurable, confidence affichée, jamais d'application automatique. L'utilisateur reste arbitre. |
| Régression involontaire sur le pipeline v1.0 | Faible | Élevé | Module `src/scan/` orthogonal, pas d'import dans `src/pipeline/`. Test de non-régression : tous les tests v1.0 doivent passer. |
| Détection de langue imprécise sur petits échantillons | Moyen | Faible | Échantillonner au moins 50 mails ou 10% (le plus élevé). Si moins de 50 mails, ne pas suggérer de prompt language. |
| Privacy : lecture inattendue du corps | Faible | Très haut | Garde-fou de code : aucun accès `body_*` dans le module `scan/`. Audit explicite à la PR. |
| Migration SQLite échoue sur base existante | Faible | Moyen | Migration v002 idempotente (`CREATE TABLE IF NOT EXISTS`). Test sur base v0.1.0 réelle avant release. |

## 9. Non-scope v1.1

- ❌ Application automatique des suggestions (jamais, principe inaliénable)
- ❌ Mode interactif TUI (`--interactive` avec questions Y/n) — repoussé v1.2
- ❌ Scan automatique périodique (cron) — repoussé v1.2, manuel uniquement
- ❌ Scan multi-utilisateur (cohérent avec v1.0 mono-utilisateur)
- ❌ Suggestions de webhooks (la couche webhook reste à configurer manuellement)
- ❌ Suggestions de prompts LLM complets (seulement un hint de langue dominante)
- ❌ Apprentissage incrémental (le scan est ponctuel, pas continu)
- ❌ Suggestions basées sur le contenu des mails (uniquement headers)

## 10. Roadmap post-v1.1 (indicatif)

- **v1.2** : mode interactif `berger scan --interactive` (TUI ratatui), scan périodique configurable, comparaison de scans (diff entre deux scans pour voir l'évolution de l'inbox)
- **v1.3** : scan AI-assisted (un appel LLM "one-shot" sur échantillon pour suggérer des catégories en langage naturel, opt-in explicite)
- **v2** : intégration native dans Twake.ai, scan en arrière-plan avec recommandations push à l'utilisateur

---

*Document additif. Ne modifie pas le PRD v1.0. À implémenter après release `v0.1.0` taguée et stabilisée 7 jours en production.*
