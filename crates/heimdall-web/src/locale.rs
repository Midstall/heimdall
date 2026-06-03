use axum::http::{HeaderMap, header};
use heimdall_i18n::Locale;

/// Pick a locale from a `?lang=` query or the Accept-Language header.
/// Falls back to English if nothing matches. Pulled into its own
/// module so future locale-resolution changes (cookie, user pref) only
/// touch one file.
pub fn resolve_locale(query: &Option<String>, headers: &HeaderMap) -> Locale {
    if let Some(s) = query.as_deref() {
        if let Some(l) = Locale::from_tag(s) {
            return l;
        }
    }
    if let Some(val) = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
    {
        for chunk in val.split(',') {
            let tag = chunk.split(';').next().unwrap_or("").trim();
            if let Some(l) = Locale::from_tag(tag) {
                return l;
            }
        }
    }
    Locale::En
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn query_string_wins_over_accept_language() {
        let mut hdr = HeaderMap::new();
        hdr.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static("en"));
        assert_eq!(resolve_locale(&Some("ja".into()), &hdr), Locale::Ja);
    }

    #[test]
    fn accept_language_is_used_when_no_query() {
        let mut hdr = HeaderMap::new();
        hdr.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static("ja"));
        assert_eq!(resolve_locale(&None, &hdr), Locale::Ja);
    }

    #[test]
    fn falls_back_to_english() {
        assert_eq!(resolve_locale(&None, &HeaderMap::new()), Locale::En);
    }

    #[test]
    fn malformed_query_falls_through_to_accept_language() {
        let mut hdr = HeaderMap::new();
        hdr.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static("ja"));
        assert_eq!(resolve_locale(&Some("nope".into()), &hdr), Locale::Ja);
    }
}
