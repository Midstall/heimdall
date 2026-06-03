// Small DOM/string helpers shared across views.

import type { ValueRepr } from "./types.js";

export function escapeHtml(s: unknown): string {
  return String(s).replace(/[&<>"']/g, (c) => {
    switch (c) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      case "'":
        return "&#39;";
      default:
        return c;
    }
  });
}

export function hexBytes(bytes: number[]): string {
  return bytes.map((b) => b.toString(16).padStart(2, "0")).join("");
}

export function shortId(s: string | null | undefined): string {
  return (s || "").slice(0, 8);
}

// Backend stamps UTC RFC3339; user sees their browser-local time.
// Both formatters built once and reused per render to avoid Intl
// init churn. Log lines want sub-second precision (the order of
// events inside one second matters); job-table cells want a coarser
// date+time-of-day pairing because the column shows when a job was
// created, not when an event landed.
const LOG_TIME_FMT = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  fractionalSecondDigits: 3,
  hour12: false,
});

const TABLE_TIME_FMT = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});

export function fmtTime(iso: string | null | undefined): string {
  if (!iso) return "-";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  return TABLE_TIME_FMT.format(new Date(t));
}

export function formatLogTime(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  return LOG_TIME_FMT.format(new Date(t));
}

export function formatValueRepr(repr: ValueRepr | null | undefined): string {
  if (!repr || typeof repr !== "object") return String(repr);
  if (repr.type === "u64" && typeof repr.value === "number") {
    return `0x${repr.value.toString(16)}`;
  }
  if (repr.type === "bool") return String(repr.value);
  if (repr.type === "bytes" && Array.isArray(repr.value)) {
    return hexBytes(repr.value as number[]);
  }
  return JSON.stringify(repr);
}

// Sort key for register-state rows: `pc` first, `xN` by numeric N,
// everything else alphabetically after. Used by renderState so the
// table reads x0, x1, ..., x31 instead of the lex-sorted order
// (x1, x10, x11, ..., x19, x2, ...) JSON object iteration would give.
export type RegSortKey = readonly [number, number, string];

export function regRowOrder(name: string): RegSortKey {
  if (name === "pc") return [0, 0, ""] as const;
  const m = /^x(\d+)$/.exec(name);
  if (m && m[1] !== undefined) {
    const n = Number(m[1]);
    if (Number.isInteger(n) && n >= 0) return [1, n, ""] as const;
  }
  return [2, 0, name] as const;
}
