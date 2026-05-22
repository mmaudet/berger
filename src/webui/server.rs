// Berger: open-source email triage daemon.
// Copyright (C) 2026 Michel-Marie Maudet
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The Axum HTTP server: shared state, the router, and the four route
//! handlers (PRD §5.7).

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::storage::database::Database;
use crate::webui::error::WebError;
use crate::webui::queries;
use crate::webui::static_assets::STYLESHEET;
use crate::webui::templates::{ConfigPage, ExplainPage, IndexPage, RecentPage};

/// How many rows the `/recent` page shows (PRD §5.7 asks for the last 50).
const RECENT_LIMIT: usize = 50;

/// State shared by every WebUI handler.
///
/// The sidecar's `rusqlite::Connection` is not `Sync`, so it is guarded by a
/// `Mutex`; WebUI queries are short and read-only, so the lock is held only
/// for the duration of a request. The raw configuration text is kept so the
/// `/config` page can render it (with secrets redacted).
#[derive(Clone)]
pub struct AppState {
    /// The sidecar database, shared with the triage loop.
    database: Arc<Mutex<Database>>,
    /// The raw `berger.yaml` text, as loaded at startup.
    raw_config: Arc<str>,
}

impl AppState {
    /// Builds the shared state from the sidecar handle and the raw config.
    pub fn new(database: Arc<Mutex<Database>>, raw_config: impl Into<Arc<str>>) -> Self {
        Self {
            database,
            raw_config: raw_config.into(),
        }
    }
}

/// Builds the WebUI router: the four pages plus the static stylesheet.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/recent", get(recent))
        .route("/explain/{message_id}", get(explain))
        .route("/config", get(config))
        .route("/static/berger.css", get(stylesheet))
        .with_state(state)
}

/// Serves the WebUI on `0.0.0.0:port` until the process exits (PRD §5.7
/// fixes the port at 7000).
///
/// # Errors
/// Returns an error if the port cannot be bound or the server stops with a
/// transport error.
pub async fn serve(state: AppState, port: u16) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, "webui listening");
    axum::serve(listener, router(state)).await
}

/// `GET /` — the statistics dashboard.
async fn index(State(state): State<AppState>) -> Result<IndexPage, WebError> {
    let stats = with_connection(&state, queries::dashboard_stats)?;
    Ok(IndexPage::new(stats))
}

/// `GET /recent` — the most recently triaged messages.
async fn recent(State(state): State<AppState>) -> Result<RecentPage, WebError> {
    let messages = with_connection(&state, |conn| queries::recent_messages(conn, RECENT_LIMIT))?;
    Ok(RecentPage::new(messages))
}

/// `GET /explain/<id>` — the full triage of one message.
async fn explain(
    State(state): State<AppState>,
    Path(message_id): Path<String>,
) -> Result<ExplainPage, WebError> {
    let explanation = with_connection(&state, |conn| {
        queries::message_explanation(conn, &message_id)
    })?;
    match explanation {
        Some(explanation) => Ok(ExplainPage::new(explanation)),
        None => Err(WebError::MessageNotFound(message_id)),
    }
}

/// `GET /config` — the active configuration, secrets redacted.
async fn config(State(state): State<AppState>) -> ConfigPage {
    ConfigPage::new(&state.raw_config)
}

/// `GET /static/berger.css` — the WebUI stylesheet.
async fn stylesheet() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLESHEET,
    )
        .into_response()
}

/// Runs `query` against the locked sidecar connection.
///
/// The mutex is only ever locked here, for a single short read; a poisoned
/// lock (a previous handler panicked mid-query) becomes a 500.
fn with_connection<T>(
    state: &AppState,
    query: impl FnOnce(&rusqlite::Connection) -> Result<T, rusqlite::Error>,
) -> Result<T, WebError> {
    let database = state.database.lock().map_err(|_| WebError::LockPoisoned)?;
    query(database.connection()).map_err(WebError::Database)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::llm_decisions::LlmDecision;
    use crate::storage::processed_messages::ProcessedMessage;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `Router::oneshot`

    const SAMPLE_YAML: &str = "\
bichon:
  base_url: \"https://bichon.example\"
  api_token: \"tok-super-secret\"
database:
  path: \"berger.db\"
accounts:
  - name: \"LINAGORA\"
    bichon_account_id: \"111\"
    imap:
      host: \"imap.example\"
      user: \"berger\"
      password: \"imap-super-secret\"
";

    /// A router over an in-memory sidecar seeded with one processed message.
    fn test_app() -> Router {
        let db = Database::open(":memory:").unwrap();
        let account_id = db.accounts().insert("LINAGORA", "bichon-1").unwrap();
        db.processed_messages()
            .record(&ProcessedMessage {
                message_id: "<m1@test>".to_string(),
                account_id,
                bichon_uri: None,
                subject: Some("Quarterly report".to_string()),
                from_email: Some("alice@test".to_string()),
                from_name: Some("Alice".to_string()),
                date: None,
                berger_version: "0.0.1".to_string(),
                config_hash: "cfg".to_string(),
            })
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO applied_tags (message_id, tag, applied_at) \
                 VALUES ('<m1@test>', 'cat/work', CURRENT_TIMESTAMP)",
                [],
            )
            .unwrap();
        db.llm_decisions()
            .record(&LlmDecision {
                message_id: "<m1@test>".to_string(),
                model: "mistral-small".to_string(),
                prompt_hash: "h1".to_string(),
                prompt_text: "classify this email".to_string(),
                response_json: r#"{"category":"work"}"#.to_string(),
                tokens_input: Some(120),
                tokens_output: Some(18),
                latency_ms: Some(210),
                cost_usd: Some(0.0012),
            })
            .unwrap();
        let state = AppState::new(Arc::new(Mutex::new(db)), SAMPLE_YAML);
        router(state)
    }

    async fn body_text(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    async fn get(app: &Router, uri: &str) -> Response {
        app.clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn index_page_renders_with_stats() {
        let response = get(&test_app(), "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = body_text(response).await;
        assert!(html.contains("Stats"));
        assert!(html.contains("Cache hit rate"));
        // One message is processed; the total card must show it.
        assert!(html.contains("Total"));
    }

    #[tokio::test]
    async fn recent_page_lists_a_processed_message() {
        let response = get(&test_app(), "/recent").await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = body_text(response).await;
        assert!(html.contains("Quarterly report"));
        assert!(html.contains("Alice"));
        assert!(html.contains("cat/work"));
    }

    #[tokio::test]
    async fn explain_page_shows_the_full_chain() {
        let response = get(&test_app(), "/explain/%3Cm1@test%3E").await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = body_text(response).await;
        assert!(html.contains("Quarterly report"));
        assert!(html.contains("cat/work"));
        assert!(html.contains("mistral-small"));
        assert!(html.contains("classify this email"));
    }

    #[tokio::test]
    async fn explain_page_returns_404_for_an_unknown_message() {
        let response = get(&test_app(), "/explain/%3Cghost@test%3E").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn config_page_redacts_secrets() {
        let response = get(&test_app(), "/config").await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = body_text(response).await;
        // The configuration's shape is shown.
        assert!(html.contains("base_url"));
        assert!(html.contains("api_token"));
        assert!(html.contains("password"));
        // The IMAP password and the API token must never reach the client.
        assert!(!html.contains("imap-super-secret"));
        assert!(!html.contains("tok-super-secret"));
        // The redaction marker is present (Askama escapes the angle brackets).
        assert!(html.contains("redacted"));
    }

    #[tokio::test]
    async fn stylesheet_is_served_as_css() {
        let response = get(&test_app(), "/static/berger.css").await;
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(content_type.starts_with("text/css"));
        assert!(body_text(response).await.contains("--bg"));
    }

    #[tokio::test]
    async fn an_unknown_route_is_404() {
        let response = get(&test_app(), "/does-not-exist").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
