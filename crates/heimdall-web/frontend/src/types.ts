// Wire-shape types. Where a generated ts-rs binding exists under
// the rolldown `bindings/` alias (resolved to `$OUT_DIR/bindings/`
// by the heimdall-web build script), we re-export it as the
// source of truth so the Rust type stays single-sourced. The
// remaining interfaces are hand-written shims for response
// wrappers and types whose Rust counterparts aren't annotated yet
// (chrono / uuid / response envelopes carrying `serde(flatten)`).
//
// Bindings regenerate on every `cargo build`; there's no separate
// regen step and nothing under `bindings/` is checked in.

// ---- Auto-generated, re-exported ----

export type { AboutFeatures } from "bindings/AboutFeatures.js";
export type { AboutInfo } from "bindings/AboutInfo.js";
export type { CampaignState } from "bindings/CampaignState.js";
export type { CampaignTemplate } from "bindings/CampaignTemplate.js";
export type { DisasmLine } from "bindings/DisasmLine.js";
export type { DisasmListing } from "bindings/DisasmListing.js";
export type { Isa } from "bindings/Isa.js";
export type { JobKind } from "bindings/JobKind.js";
export type { JobState } from "bindings/JobState.js";
export type { LogLevel } from "bindings/LogLevel.js";
export type { SnapshotSourceTag } from "bindings/SnapshotSourceTag.js";
export type { State } from "bindings/State.js";
export type { ValueRepr } from "bindings/ValueRepr.js";
export type { VerdictSummary } from "bindings/VerdictSummary.js";

import type { JobKind } from "bindings/JobKind.js";
import type { JobState } from "bindings/JobState.js";
import type { LogLevel } from "bindings/LogLevel.js";
import type { SnapshotSourceTag } from "bindings/SnapshotSourceTag.js";
import type { State as RustState } from "bindings/State.js";

// ---- Frontend-local shims ----
//
// These mirror response-wrapper / WebSocket-frame shapes that the
// daemon serializes via `serde(flatten)` or `untagged` patterns
// ts-rs doesn't reproduce cleanly. Keep narrow and grep-stable so
// the next pass can replace them with auto-generated equivalents
// once the underlying Rust types are annotated.

export interface JobLogEntry {
  level: LogLevel;
  message: string;
  stage: string | null;
  i18n_key: string | null;
  i18n_args: Record<string, string | number | boolean> | null;
  // RFC3339 UTC timestamp.
  ts: string;
}

export interface JobLogsResponse {
  logs: Array<JobLogEntry & { id?: number }>;
}

/// Frontend-facing alias matching the historic name we used before
/// the snapshot source enum got auto-generated.
export type SourceTag = SnapshotSourceTag;

export interface DutSnapshot {
  source: SourceTag;
  ts: string;
  job: string | null;
  state: RustState;
}

export interface EffectiveIsa {
  xlen: number;
  extensions: string[];
  source: "probe" | "config";
}

export interface DutRecord {
  id: string;
  kind: string;
  chip_serial: string | null;
  jtag: { driver: string } | null;
  connection_status?: "connected" | "disconnected" | "unknown";
  snapshots?: DutSnapshot[];
  netlist?: unknown;
  /// Present when the daemon resolved a probe or config ISA for the
  /// DUT. Absent means base/default; the UI renders an `unknown` chip.
  effective_isa?: EffectiveIsa;
}

export interface LeaseSummary {
  dut: string;
  holder: string | null;
}

export interface DutsResponse {
  duts: DutRecord[];
  leases?: LeaseSummary[];
}

/// Frontend-facing `VerdictKind` retained for compatibility with the
/// per-job state-class CSS classes (`verdict-pass`, ...). Mirrors
/// `VerdictSummary["kind"]`.
export type VerdictKind = "pass" | "fail" | "skip" | "error";

/// The web UI renders job rows with a tag-tag-tag string mash-up, so
/// we accept both the auto-generated discriminated union and a
/// lighter shape that just exposes the `state` tag + optional detail.
export type JobStateInfo = JobState | { state: string; detail?: unknown };

export interface Job {
  id: string;
  dut: string;
  kind: JobKind;
  state: JobStateInfo | null;
  created_at: string;
}

export interface JobsResponse {
  jobs: Job[];
  /// Total rows matching the active filter, ignoring `limit` /
  /// `offset`. Drives the pager's "page X of N" math.
  total: number;
}

export interface Campaign {
  id: string;
  dut: string;
  template: { kind: string };
  state: { state: string } | null;
  chip_serial: string | null;
}

export interface CampaignsResponse {
  campaigns: Campaign[];
}

export interface EventBase {
  kind: string;
  ts?: string;
  job?: string;
}

export interface JobLogEvent extends EventBase {
  kind: "job-log";
  job: string;
  ts: string;
  level: LogLevel;
  message: string;
  stage: string | null;
  i18n_key: string | null;
  i18n_args: Record<string, string | number | boolean> | null;
}

export interface DutStateSnapshotEvent extends EventBase {
  kind: "dut-state-snapshot";
  dut: string;
  ts: string;
  source: SourceTag;
  state: RustState;
}

export type WsEvent = JobLogEvent | DutStateSnapshotEvent | EventBase;

import type { DisasmListing } from "bindings/DisasmListing.js";

/// Frontend-only union: the disasm cache either holds a listing
/// (with an optional `iter` field the daemon adds for fuzz jobs) or
/// an error envelope. The error envelope mirrors the daemon's
/// `DisasmErrorBody` shape: an English `error` fallback plus an
/// optional `i18n_key` + `i18n_args` pair the frontend prefers when
/// it's set so operators see the message in their locale.
export type DisasmCacheEntry =
  | (DisasmListing & { iter?: number })
  | {
      error: string;
      i18n_key?: string;
      i18n_args?: Record<string, string>;
    };
