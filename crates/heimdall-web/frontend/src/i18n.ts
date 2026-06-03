// i18n catalog + translation helpers. Catalog ships from
// `/i18n.json?lang=<tag>` and is held module-level; callers stay
// async-agnostic by reading through `t()` / `tr()` after
// `initI18n()` has resolved.

type Catalog = Record<string, string>;

type I18nArgs = Record<string, string | number | boolean>;

let catalog: Catalog = {};
let activeLocale: string = "en";

function detectLocale(): string {
  const params = new URLSearchParams(location.search);
  const fromQuery = params.get("lang");
  if (fromQuery) return fromQuery;
  try {
    const stored = localStorage.getItem("heimdall.lang");
    if (stored) return stored;
  } catch (_) {
    // localStorage may be disabled (private browsing); fall through.
  }
  return navigator.language || "en";
}

/// Translate `key`, falling back to `fallback` (or the key itself)
/// when the catalog is empty or doesn't include it. Used everywhere
/// the UI needs a localised string.
export function t(key: string, fallback?: string): string {
  return catalog[key] || fallback || key;
}

/// Translate + substitute `{name}` placeholders from `args`.
export function tr(key: string, args?: I18nArgs): string {
  return substitute(t(key), args);
}

export function substitute(tmpl: string, args?: I18nArgs): string {
  if (!args) return tmpl;
  let s = tmpl;
  for (const name of Object.keys(args)) {
    s = s.split(`{${name}}`).join(String(args[name]));
  }
  return s;
}

function applyStaticTranslations(): void {
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    if (!key) return;
    const translated = catalog[key];
    if (translated) el.textContent = translated;
  });
  document.documentElement.setAttribute("lang", activeLocale);
}

/// Fetch the catalog for the detected locale and apply static
/// translations to any `[data-i18n]` element. Awaitable so the
/// caller can defer first paint until the right locale is loaded.
export async function initI18n(): Promise<void> {
  const lang = detectLocale();
  try {
    const r = await fetch(`/i18n.json?lang=${encodeURIComponent(lang)}`);
    if (r.ok) {
      const body = (await r.json()) as Catalog & { _locale?: string };
      catalog = body;
      activeLocale = body._locale || "en";
      try {
        localStorage.setItem("heimdall.lang", activeLocale);
      } catch (_) {
        // ignore
      }
    }
  } catch (_) {
    // Network failure: leave catalog empty. Keys surface as fallback text.
  }
  applyStaticTranslations();
}
