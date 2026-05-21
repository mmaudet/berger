# CLAUDE.md — Briefing initial pour l'agent

Ce fichier est ton instruction principale. Lis-le **avant tout autre fichier**. Il oriente toute la session.

---

## 1. Mission

Tu construis **Berger v1.0**, un démon Rust de triage email open-source. Le document de référence absolu est `docs/PRD.md` — tu dois l'avoir lu en intégralité avant d'écrire la moindre ligne de code.

L'auteur du projet est **Michel-Marie Maudet** (Directeur Général de LINAGORA). Repo : `github.com/mmaudet/berger`. Licence : **AGPLv3**. Délai cible : 2 semaines de sprint dense.

Berger se positionne comme l'*afew de 2026* : un trieur d'email open-source moderne, en Rust, branché sur un archiveur (Bichon) en amont, qui pose des tags via filtres natifs + LLM et matérialise le tri via folders IMAP visibles dans tous les clients mail.

## 2. Hiérarchie des sources de vérité

Dans cet ordre, du plus au moins prioritaire :

1. **Ce fichier `CLAUDE.md`** : règles de comportement, scope, garde-fous
2. **`docs/PRD.md`** : spécification fonctionnelle et technique complète
3. **Tes propres choix d'implémentation** : seulement pour les détails non spécifiés (nommage local de variables, structure de fonctions internes, etc.)

**Si tu détectes une contradiction** entre ces sources, ou si une décision t'oblige à sortir du scope, **stoppe et demande**. Ne tranche jamais seul sur une dérive de scope.

## 3. Règles dures non-négociables

Ces règles ont priorité sur tout, y compris sur la performance, l'élégance, ou ton intuition de "ça serait mieux comme ça".

### 3.1 Scope verrouillé

- Tout ce qui est listé en **§ 6 Non-scope** du PRD est **interdit** au MVP. Aucune exception.
- Aucune anticipation de fonctionnalités v1.1 ou v2. Pas de "je prépare le terrain pour plus tard" qui complique le MVP.
- Si tu te dis *"ce serait bien d'ajouter…"*, c'est probablement hors scope. Demande avant.

### 3.2 Les trois règles dures de cohérence Bichon (§ 5.10 du PRD)

Ces règles sont la **première chose à implémenter et à tester** car leur absence cause un bouclage infini :

1. **Filtrage `Berger/*` en lecture** : tout mail dont le dossier source commence par `Berger/` est ignoré, sans exception. Implémenté dans le module `ingest`.
2. **Idempotence par Message-ID** : avant tout traitement, lookup dans `processed_messages`. Si présent, skip. Implémenté dans le module `pipeline`.
3. **`ensure_folder_exists`** avant chaque action `copy_to`/`move_to`. Implémenté dans le module `actions`.

Tests d'intégration dédiés obligatoires pour les trois (cf. § 10 du PRD).

### 3.3 Sécurité destructive

- **Aucun `EXPUNGE` global**. Seul `move_to` peut expunger l'UID concerné, après confirmation que le COPY est passé.
- **Aucune suppression définitive** (`delete: true` ou équivalent). Hors scope MVP, point final.
- **Aucune modification de contenu RFC822** des mails sur le serveur source. Pas d'APPEND modifié, pas de header injecté dans le mail.

### 3.4 Licence et attribution

- Chaque fichier source Rust (`.rs`) doit commencer par l'en-tête AGPLv3 standard avec le copyright `Copyright (C) 2026 Michel-Marie Maudet`.
- `LICENSE` à la racine = texte complet AGPLv3.
- `README.md` mentionne la licence en évidence.
- Aucune dépendance avec une licence incompatible AGPL (pas de GPL strict, pas de licence propriétaire). Vérifie via `cargo deny`.

### 3.5 Aucune télémétrie

Berger ne phone home jamais. Aucun call HTTP autre que ceux explicitement déclarés par l'utilisateur dans son YAML (Bichon, LLM, webhooks). Aucun crash report automatique.

## 4. Standards de code Rust

### 4.1 Toolchain et qualité

- Rust **stable**, édition **2024**
- `rustfmt` appliqué systématiquement (config par défaut)
- `cargo clippy --all-targets --all-features -- -D warnings` doit passer
- `cargo test` doit passer
- `cargo doc --no-deps` doit générer **sans warning**
- `cargo deny check` doit passer (licences + sécurité dépendances)

### 4.2 Conventions

- **Error handling** : `thiserror` pour les erreurs du domaine, `anyhow` uniquement aux frontières (CLI, main). Aucun `unwrap()` en code de production sauf justifié par un commentaire `// SAFETY:` explicite.
- **Logging** : `tracing` exclusivement. Aucun `println!` ou `eprintln!` hors code de la CLI elle-même. Niveau de log par défaut : INFO. Champs structurés (`message_id = %id`) plutôt que interpolation string.
- **Async** : `tokio` comme runtime unique. Pas de mélange avec `async-std` ou autre.
- **Sérialisation** : `serde` partout. `serde_json` pour JSON, `serde_yaml` pour YAML.
- **Types publics** documentés avec doc-comments `///`. Exemples dans la doc si non trivial.
- **Nommage** : `snake_case` Rust standard. Pas d'abréviations cryptiques. Préfère `message_id` à `mid`.

### 4.3 Structure du crate

Crate binaire unique `berger` avec modules clairs :

```
src/
├── main.rs              # entry point, init logging, dispatch CLI
├── cli/                 # clap commands (run, explain, status, dry-run, export-thunderbird)
├── config/              # parsing + validation YAML
├── ingest/              # client Bichon REST + filtrage Berger/* + watermark
├── pipeline/            # orchestration + idempotence check
├── filters/             # filtres natifs (list_unsubscribe, sender_in, subject_regex, header_match)
├── llm/                 # client OpenAI-compatible + cache + schema JSON
├── tags/                # mapping classification → tags
├── actions/             # moteur d'actions IMAP + ensure_folder_exists
├── webhooks/            # POST + Handlebars templating + retry
├── storage/             # rusqlite + refinery, repository pattern, les 7 tables
├── webui/               # Axum + Askama + routes /, /recent, /explain/<id>, /config
└── observability/       # init tracing, metrics counters
```

### 4.4 Tests

- **Tests unitaires** : à côté du code dans `mod tests`, pour chaque module logique
- **Tests d'intégration** : dans `tests/`, avec serveur IMAP de dev (Greenmail via testcontainers) et serveur HTTP mock pour Bichon/LLM/webhooks (`wiremock` ou `mockito`)
- **Tests critiques obligatoires** (§ 10 du PRD) :
  - `tests/idempotence.rs` : soumettre 3 fois le même Message-ID, vérifier que les actions IMAP ne s'exécutent qu'une fois
  - `tests/berger_folder_filter.rs` : un mail provenant de `Berger/cat-work/` côté Bichon n'est jamais retraité
  - `tests/ensure_folder.rs` : supprimer un dossier puis déclencher `copy_to`, vérifier recréation + posting
  - `tests/webhook_payload.rs` : émettre un webhook, vérifier que le JSON envoyé est strictement conforme au schéma canonique du PRD § 5.6
- **Coverage** : pas d'objectif chiffré strict, mais chaque branche de logique métier critique doit avoir un test.

## 5. Workflow de développement

### 5.1 Branche unique

Travaille directement sur `main`. Pas de branches features pour ce sprint solo. Commits petits, granulaires, atomiques.

### 5.2 Commits granulaires et lisibles

**Conventional Commits** strict :

```
feat(ingest): add bichon REST client with cursor watermark
fix(actions): ensure folder is subscribed after creation
test(idempotence): cover triple-submit case
docs(yaml): add example for spam-confirme tag
chore(deps): bump async-imap to 0.10
refactor(pipeline): extract tag mapping into dedicated module
```

Un commit = une intention claire. Pas de commit fourre-tout `wip`, `stuff`, `update files`.

### 5.3 Ordre de livraison

Suivre **strictement** l'ordre des jalons du PRD § 9. Ne saute pas un jalon. Si un jalon est bloqué (par exemple parce qu'un crate Rust se comporte mal), stoppe et signale, ne contourne pas en allant en avant.

### 5.4 Pas de PR auto-mergeable

Tu ne mergues jamais. Tu commits, tu pushes, et Michel-Marie revoit avant le push suivant si nécessaire. Si tu produis 30 commits dans une session, c'est OK tant qu'ils sont propres et atomiques.

## 6. Documentation à livrer

À la fin du sprint, le repo doit contenir :

- `README.md` : pitch (3 lignes), screenshot WebUI, quickstart (Docker compose), licence, contribution rules
- `docs/PRD.md` : déjà fourni, à committer tel quel
- `docs/yaml.md` : référence complète du format `berger.yaml`, tous les filtres, toutes les actions, exemples annotés
- `docs/webhooks.md` : contrat de payload canonique + 3 cas d'usage canoniques détaillés + exemples de workflows n8n
- `docs/bichon-setup.md` : comment configurer Bichon en amont (excluded_folders, etc.)
- `docs/ops.md` : déploiement systemd, Docker, backup SQLite, logs, métriques
- `berger.example.yaml` à la racine : configuration de référence couvrant les 4 filtres natifs, les 5 primitives d'actions, 3 webhooks

**Ton du README** : technique mais accessible, marketing minimal, pas de hype, pas d'émojis sauf dans les badges en haut. S'inspirer du README de [tokio](https://github.com/tokio-rs/tokio) ou [ruff](https://github.com/astral-sh/ruff).

## 7. Garde-fous comportementaux

### 7.1 Quand stopper et demander

- Si tu hésites entre deux approches qui ont des implications visibles dans le PRD
- Si une dépendance que tu veux ajouter n'est pas listée dans le § 8 Stack technique du PRD
- Si un test échoue d'une manière que tu ne comprends pas (ne le commente jamais comme `#[ignore]`)
- Si tu te rends compte qu'un jalon va prendre plus que la journée prévue
- Si tu remarques une incohérence dans le PRD lui-même

### 7.2 Comportement interdit

- ❌ **Inventer des fonctionnalités** non spécifiées (même si "évidentes")
- ❌ **Commenter du code** pour le désactiver sans expliquer pourquoi
- ❌ **Ignorer des tests** (`#[ignore]`, `#[cfg(not(test))]` détourné)
- ❌ **Ajouter des `unwrap()` non justifiés**
- ❌ **Utiliser des `unsafe`** — Berger v1 n'a besoin d'aucun unsafe
- ❌ **Stocker des secrets en clair** dans le code, le YAML, les tests, ou les logs
- ❌ **Embarquer des fichiers binaires** > 100 KB dans le repo (sauf nécessaire pour les tests)
- ❌ **Modifier le PRD** sans autorisation explicite (le PRD est gelé sauf demande)

### 7.3 Ce qui est OK à décider seul

- Le nom interne d'une variable, d'un module privé, d'une struct interne
- L'ordre exact des champs d'une struct
- Les détails du formatting des logs (tant que c'est JSON structuré)
- Les noms de fixtures de test
- Les commentaires de code (en français accepté, anglais préféré pour les doc-comments publics)

## 8. Premier commit attendu

Au démarrage de ta première session, fais exactement et seulement ceci :

1. `git init`, `git remote add origin git@github.com:mmaudet/berger.git`
2. Crée `LICENSE` (texte AGPLv3 complet)
3. Crée `README.md` minimal (titre, pitch 3 lignes, statut "🚧 v0 en construction", licence)
4. Crée `Cargo.toml` avec :
   - `name = "berger"`
   - `version = "0.0.1"`
   - `edition = "2024"`
   - `license = "AGPL-3.0-or-later"`
   - `authors = ["Michel-Marie Maudet <michel-marie@linagora.com>"]`
   - `description = "Open-source email triage daemon, the afew of 2026"`
   - `repository = "https://github.com/mmaudet/berger"`
5. Crée la structure modules vide telle que décrite en § 4.3 (un `mod.rs` vide ou un fichier `src/<module>/mod.rs` qui compile)
6. Crée `src/main.rs` minimal qui imprime `"berger v0.0.1"` et exit 0
7. Crée `.github/workflows/ci.yml` : matrix build sur stable + clippy + fmt + test
8. Crée `.gitignore` Rust standard + `berger.db*` + `*.env`
9. Copie le PRD à `docs/PRD.md`
10. Copie ce briefing à `CLAUDE.md` (racine du repo)
11. Commit unique : `chore: initial scaffold for berger v0.0.1`
12. **Stop. Demande validation avant de passer au jalon J2.**

Après ce premier commit, tu ne fais **plus rien** sans confirmation de l'auteur. C'est volontairement bref — c'est un point de contrôle pour vérifier que la fondation est saine avant de dérouler les 13 jalons restants.

## 9. Définition de "fini" pour le sprint

Le sprint est terminé quand **tous** les items suivants sont vrais, sans exception :

- [ ] Tous les critères d'acceptation du PRD § 10 sont cochés et testés
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passe
- [ ] `cargo test` passe (unitaires + intégration)
- [ ] `cargo doc --no-deps` génère sans warning
- [ ] `cargo deny check` passe
- [ ] `docker build -t berger:0.1.0 .` réussit et `docker run` démarre le démon
- [ ] La WebUI est accessible à `:7000` et les 4 pages s'affichent correctement
- [ ] La doc (`README.md`, `docs/*.md`, `berger.example.yaml`) est complète
- [ ] Un tag git `v0.1.0` est posé
- [ ] Le release GitHub `v0.1.0` est créé avec changelog
- [ ] Le démon a tourné au moins 24h sans crash sur le compte mail réel de l'auteur

Tant qu'un seul item est faux, le sprint n'est pas terminé. Si un item devient impossible à atteindre dans le temps imparti, c'est une situation à signaler immédiatement, pas à ignorer.

## 10. Ton attitude de travail

- **Direct** : pas de "bonne nouvelle/mauvaise nouvelle". Annonce les faits.
- **Honnête** : si tu ne sais pas, dis-le. Si une dépendance se comporte bizarrement, dis-le. Si un test passe par chance, dis-le.
- **Frugal** : préfère 100 lignes de Rust lisibles à 50 lignes clever. Préfère 3 dépendances à 8.
- **Patient** : un jalon par jour, c'est le rythme. Ne précipite pas.
- **Respectueux du scope** : le PRD est gelé. Si tu dois sortir du scope, c'est par demande explicite, pas par initiative.

---

*Fin du briefing. À ce stade, tu peux lire `docs/PRD.md` et démarrer le premier commit (§ 8 ci-dessus). Bonne route.*
