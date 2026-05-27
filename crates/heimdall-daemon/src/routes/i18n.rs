//! GET /i18n.json: serves the flattened translation catalog for the
//! requested locale so the web UI can localize itself.
//!
//! Query string:
//!   ?lang=en | ja   (explicit)
//! Header (fallback):
//!   Accept-Language: ja, en;q=0.5
//!
//! Unknown locales silently fall back to English. The response includes a
//! `_locale` field with the locale that was actually served so the JS can
//! reflect it back to the user.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::Query,
    http::{HeaderMap, header},
    routing::get,
};
use heimdall_i18n::Locale;
use serde::{Deserialize, Serialize};

use crate::server::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/i18n.json", get(serve))
}

#[derive(Deserialize)]
struct LangQuery {
    lang: Option<String>,
}

#[derive(Serialize)]
struct Response {
    /// Locale code that was actually served (`"en"` or `"ja"`).
    #[serde(rename = "_locale")]
    locale: &'static str,
    /// Flat catalog: dotted keys like `tui.view.jobs` -> translated string.
    #[serde(flatten)]
    entries: BTreeMap<String, String>,
}

async fn serve(Query(q): Query<LangQuery>, headers: HeaderMap) -> Json<Response> {
    let locale = q
        .lang
        .as_deref()
        .and_then(Locale::from_tag)
        .or_else(|| accept_language(&headers))
        .unwrap_or(Locale::En);
    Json(Response {
        locale: locale.code(),
        entries: heimdall_i18n::catalog(locale),
    })
}

fn accept_language(headers: &HeaderMap) -> Option<Locale> {
    let value = headers.get(header::ACCEPT_LANGUAGE)?.to_str().ok()?;
    // Quick-and-dirty Accept-Language parse: try each comma-separated entry
    // in order, strip ;q= weighting, and accept the first known locale.
    for chunk in value.split(',') {
        let tag = chunk.split(';').next().unwrap_or("").trim();
        if let Some(loc) = Locale::from_tag(tag) {
            return Some(loc);
        }
    }
    None
}
