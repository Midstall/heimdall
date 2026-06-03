//! Axum router and request handlers.
//!
//! Generic over [`WebContext`] so the daemon (which carries the
//! storage/queue/registry stack) can mount the same router that a thin
//! preview binary uses with a stub context.

use askama::Template;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use serde::Deserialize;

use crate::locale::resolve_locale;
use crate::ssr::IndexTemplate;
use crate::trait_def::WebContext;

/// Embedded asset bundle. `build.rs` assembles `$OUT_DIR/assets/` from
/// the crate's `assets/` directory plus any generated SVGs.
#[derive(RustEmbed)]
#[folder = "$OUT_DIR/assets/"]
struct Assets;

/// Build the web router. Mounts the SSR index at `/` and the static
/// asset embed at `/assets/*path`.
pub fn router<S>() -> Router<S>
where
    S: WebContext + Clone + 'static,
{
    Router::new()
        .route("/", get(index::<S>))
        .route("/assets/*path", get(asset))
}

#[derive(Deserialize)]
struct LangQuery {
    lang: Option<String>,
}

async fn index<S>(State(app): State<S>, headers: HeaderMap, Query(q): Query<LangQuery>) -> Response
where
    S: WebContext + Clone,
{
    let locale = resolve_locale(&q.lang, &headers);
    let data = match app.build_index_data(locale).await {
        Ok(d) => d,
        Err(e) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!("index data build failed: {e}")))
                .unwrap();
        }
    };
    let ctx = IndexTemplate {
        locale: locale.code().to_string(),
        jobs: data.jobs,
        campaigns: data.campaigns,
        duts: data.duts,
        locale_kind: locale,
    };
    match ctx.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from(format!("template error: {e}")))
            .unwrap(),
    }
}

async fn asset(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(file.data.into_owned()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap(),
    }
}
