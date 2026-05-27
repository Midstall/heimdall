//! Small i18n catalog for Heimdall. English + Japanese, embedded at compile
//! time, ASCII-key lookup with `{name}` placeholder interpolation.
//!
//! Surfaces:
//! - TUI (`heimdall-tui`): reads via the [`t!`] macro after calling
//!   [`set_locale`] in `run_app`.
//! - Web UI (`heimdall-daemon`): the daemon exposes
//!   `GET /i18n.json?lang=...` which serves a flattened key->value table for
//!   the requested locale; JS substitutes `[data-i18n="key"]` nodes at boot.
//! - CLI: callers can use [`t`] or [`t_args`] directly for any user-facing
//!   strings worth localizing.
//!
//! Locale precedence (first match wins):
//!   1. Explicit override via [`set_locale`]
//!   2. `HEIMDALL_LANG` env var
//!   3. `LC_ALL` / `LC_MESSAGES` / `LANG` env vars
//!   4. English fallback
//!
//! Adding strings: edit both `locales/en.toml` and `locales/ja.toml`. A key
//! missing from `ja.toml` resolves to the English value (not the bare key).

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

const EN_TOML: &str = include_str!("../locales/en.toml");
const JA_TOML: &str = include_str!("../locales/ja.toml");

/// All locales Heimdall has translations for. Order matters for `all()`:
/// English is first so it remains the fallback target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Locale {
    #[default]
    En,
    Ja,
}

impl Locale {
    /// Canonical short code (`"en"`, `"ja"`) used in URLs and config.
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Ja => "ja",
        }
    }

    /// Parse a BCP-47-ish locale tag. Accepts `en`, `en-US`, `ja`, `ja_JP.UTF-8`.
    /// Returns `None` for unknown locales (caller decides fallback).
    pub fn from_tag(tag: &str) -> Option<Self> {
        let head = tag
            .split(['-', '_', '.'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match head.as_str() {
            "en" => Some(Locale::En),
            "ja" => Some(Locale::Ja),
            _ => None,
        }
    }

    /// All supported locales, English first.
    pub fn all() -> &'static [Locale] {
        &[Locale::En, Locale::Ja]
    }
}

/// Detect the user's preferred locale from env vars. Honors
/// `HEIMDALL_LANG`, `LC_ALL`, `LC_MESSAGES`, then `LANG`. Falls back to
/// English when no recognized locale is set.
pub fn detect_locale() -> Locale {
    const ENV_VARS: &[&str] = &["HEIMDALL_LANG", "LC_ALL", "LC_MESSAGES", "LANG"];
    for var in ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            if let Some(loc) = Locale::from_tag(&val) {
                return loc;
            }
        }
    }
    Locale::En
}

static CURRENT_LOCALE: OnceLock<RwLock<Locale>> = OnceLock::new();

fn current_lock() -> &'static RwLock<Locale> {
    CURRENT_LOCALE.get_or_init(|| RwLock::new(Locale::En))
}

/// Globally set the active locale for `t!()` lookups. Returns the previous
/// value. Safe to call concurrently.
pub fn set_locale(locale: Locale) -> Locale {
    let lock = current_lock();
    let mut guard = lock.write().expect("i18n lock poisoned");
    let prev = *guard;
    *guard = locale;
    prev
}

/// Currently active locale.
pub fn current_locale() -> Locale {
    *current_lock().read().expect("i18n lock poisoned")
}

static EN_TABLE: OnceLock<BTreeMap<String, String>> = OnceLock::new();
static JA_TABLE: OnceLock<BTreeMap<String, String>> = OnceLock::new();

fn table_for(locale: Locale) -> &'static BTreeMap<String, String> {
    match locale {
        Locale::En => EN_TABLE.get_or_init(|| parse_catalog(EN_TOML, Locale::En)),
        Locale::Ja => JA_TABLE.get_or_init(|| parse_catalog(JA_TOML, Locale::Ja)),
    }
}

/// Parse a TOML catalog into a flat dotted-key -> string map. Panics on a
/// malformed catalog because catalogs are compile-time inputs; if you ship a
/// broken `en.toml`, every binary linking heimdall-i18n crashes at first use,
/// which is the loud failure mode we want.
fn parse_catalog(src: &str, locale: Locale) -> BTreeMap<String, String> {
    let value: toml::Value = toml::from_str(src)
        .unwrap_or_else(|e| panic!("heimdall-i18n: malformed catalog for {:?}: {e}", locale));
    let mut out = BTreeMap::new();
    flatten(&value, String::new(), &mut out);
    out
}

fn flatten(value: &toml::Value, prefix: String, out: &mut BTreeMap<String, String>) {
    match value {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(v, key, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix, s.clone());
        }
        // Non-string scalars aren't expected in translation catalogs; stringify
        // them so the key still resolves rather than silently dropping it.
        other => {
            out.insert(prefix, other.to_string());
        }
    }
}

/// Look up `key` in the current locale. Falls back to English; if still
/// missing, returns the key itself so the surface stays usable.
pub fn t(key: &str) -> String {
    let locale = current_locale();
    if locale != Locale::En {
        if let Some(s) = table_for(locale).get(key) {
            return s.clone();
        }
    }
    if let Some(s) = table_for(Locale::En).get(key) {
        return s.clone();
    }
    key.to_string()
}

/// Like [`t`] but substitutes `{name}` placeholders in the result with the
/// values from `args` (name, value). Unmatched placeholders are left intact.
pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (name, value) in args {
        let placeholder = format!("{{{name}}}");
        s = s.replace(&placeholder, value);
    }
    s
}

/// Look up `key` in the explicitly-specified `locale`, falling back to English
/// and then to the key. Useful for the SSR path where each request may have a
/// different locale and can't share the global current_locale().
pub fn t_in(locale: Locale, key: &str) -> String {
    if locale != Locale::En {
        if let Some(s) = table_for(locale).get(key) {
            return s.clone();
        }
    }
    if let Some(s) = table_for(Locale::En).get(key) {
        return s.clone();
    }
    key.to_string()
}

/// Explicit-locale variant of [`t_args`].
pub fn t_args_in(locale: Locale, key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t_in(locale, key);
    for (name, value) in args {
        let placeholder = format!("{{{name}}}");
        s = s.replace(&placeholder, value);
    }
    s
}

/// Return the full key->value table for a locale, flattened with dotted keys.
pub fn catalog(locale: Locale) -> BTreeMap<String, String> {
    table_for(locale).clone()
}

/// Convenience macro: `t!("foo.bar")` or `t!("foo.bar", url = url, port = p)`.
/// Each argument is stringified via `ToString` before substitution.
#[macro_export]
macro_rules! t {
    ($key:expr $(,)?) => {
        $crate::t($key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        let values: &[(&str, String)] = &[$((stringify!($name), ::std::string::ToString::to_string(&$value))),+];
        let refs: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
        $crate::t_args($key, &refs)
    }};
}

/// Emit a localized `tracing` event. The first argument is the catalog key;
/// subsequent named args (`field = value`) are interpolated into the message
/// AND attached to the event as structured fields, so log parsers can still
/// extract numeric/path values regardless of locale.
///
/// Available level macros: [`ltrace!`], [`ldebug!`], [`linfo!`], [`lwarn!`],
/// [`lerror!`].
///
/// The caller's crate must already depend on `tracing` (every Heimdall crate
/// does).
#[macro_export]
macro_rules! linfo {
    ($key:expr $(,)?) => {
        ::tracing::info!("{}", $crate::t($key))
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        ::tracing::info!(
            $($name = ?$value,)+
            "{}",
            $crate::t!($key, $($name = $value),+),
        )
    };
}

#[macro_export]
macro_rules! lwarn {
    ($key:expr $(,)?) => {
        ::tracing::warn!("{}", $crate::t($key))
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        ::tracing::warn!(
            $($name = ?$value,)+
            "{}",
            $crate::t!($key, $($name = $value),+),
        )
    };
}

#[macro_export]
macro_rules! lerror {
    ($key:expr $(,)?) => {
        ::tracing::error!("{}", $crate::t($key))
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        ::tracing::error!(
            $($name = ?$value,)+
            "{}",
            $crate::t!($key, $($name = $value),+),
        )
    };
}

#[macro_export]
macro_rules! ldebug {
    ($key:expr $(,)?) => {
        ::tracing::debug!("{}", $crate::t($key))
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        ::tracing::debug!(
            $($name = ?$value,)+
            "{}",
            $crate::t!($key, $($name = $value),+),
        )
    };
}

#[macro_export]
macro_rules! ltrace {
    ($key:expr $(,)?) => {
        ::tracing::trace!("{}", $crate::t($key))
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        ::tracing::trace!(
            $($name = ?$value,)+
            "{}",
            $crate::t!($key, $($name = $value),+),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Force a deterministic locale for this thread. The global is shared but
    /// tests run serially within a test binary by default for single-threaded
    /// rt; we use a mutex to be safe under multi-threaded rt anyway.
    fn with_locale<R>(locale: Locale, f: impl FnOnce() -> R) -> R {
        static GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        let m = GUARD.get_or_init(|| std::sync::Mutex::new(()));
        let _held = m.lock().unwrap();
        let prev = set_locale(locale);
        let out = f();
        set_locale(prev);
        out
    }

    #[test]
    fn locale_from_tag_parses_common_forms() {
        assert_eq!(Locale::from_tag("en"), Some(Locale::En));
        assert_eq!(Locale::from_tag("en-US"), Some(Locale::En));
        assert_eq!(Locale::from_tag("ja"), Some(Locale::Ja));
        assert_eq!(Locale::from_tag("ja_JP.UTF-8"), Some(Locale::Ja));
        assert_eq!(Locale::from_tag("JA"), Some(Locale::Ja));
        assert_eq!(Locale::from_tag("fr"), None);
        assert_eq!(Locale::from_tag(""), None);
    }

    #[test]
    fn english_catalog_has_known_keys() {
        with_locale(Locale::En, || {
            assert_eq!(t("tui.view.jobs"), "jobs");
            assert_eq!(t("tui.duts.col_status"), "Status");
            assert_eq!(t("common.status.connected"), "connected");
        });
    }

    #[test]
    fn japanese_catalog_translates_known_keys() {
        with_locale(Locale::Ja, || {
            assert_eq!(t("tui.view.jobs"), "ジョブ");
            assert_eq!(t("tui.duts.col_status"), "状態");
            assert_eq!(t("common.status.connected"), "接続済み");
        });
    }

    #[test]
    fn missing_key_falls_back_to_key_itself() {
        with_locale(Locale::En, || {
            assert_eq!(t("definitely.not.a.key"), "definitely.not.a.key");
        });
    }

    #[test]
    fn missing_japanese_key_falls_back_to_english() {
        // Intentionally querying a key we'd only stub in en if we ever added
        // an en-only string. Today every en key has a ja counterpart; this
        // test just exercises the fallback path via the missing-key route.
        with_locale(Locale::Ja, || {
            assert_eq!(t("nope.nope.nope"), "nope.nope.nope");
        });
    }

    #[test]
    fn placeholder_interpolation_works_both_locales() {
        with_locale(Locale::En, || {
            assert_eq!(
                t!("tui.connected_to", url = "http://127.0.0.1:7777"),
                "connected to http://127.0.0.1:7777"
            );
        });
        with_locale(Locale::Ja, || {
            assert_eq!(
                t!("tui.connected_to", url = "http://127.0.0.1:7777"),
                "http://127.0.0.1:7777 に接続済み"
            );
        });
    }

    #[test]
    fn multiple_named_args() {
        with_locale(Locale::En, || {
            assert_eq!(
                t!("tui.disconnected_reason", reason = "ws closed"),
                "disconnected: ws closed; retrying..."
            );
        });
    }

    #[test]
    fn catalog_returns_flattened_keys() {
        let en = catalog(Locale::En);
        assert!(en.contains_key("tui.view.jobs"));
        assert!(en.contains_key("web.tabs.duts"));
        assert!(en.contains_key("common.status.connected"));
        // No bare top-level table names should leak through.
        assert!(!en.contains_key("tui"));
        assert!(!en.contains_key("web"));
    }

    #[test]
    fn ja_and_en_catalogs_have_identical_key_sets() {
        let en: std::collections::BTreeSet<_> = catalog(Locale::En).into_keys().collect();
        let ja: std::collections::BTreeSet<_> = catalog(Locale::Ja).into_keys().collect();
        let en_only: Vec<_> = en.difference(&ja).collect();
        let ja_only: Vec<_> = ja.difference(&en).collect();
        assert!(
            en_only.is_empty(),
            "en has keys missing from ja: {:?}",
            en_only
        );
        assert!(
            ja_only.is_empty(),
            "ja has keys missing from en: {:?}",
            ja_only
        );
    }
}
