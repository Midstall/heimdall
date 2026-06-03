// Entry point. Boots i18n, attaches view tabs, kicks off polling, and
// wires the WebSocket event stream. The big sub-responsibilities
// (view rendering, per-job detail panel, disasm panel) live inline
// for now. Future split can extract `views/` modules once any one of
// them grows past its current size.

import {
  escapeHtml,
  fmtTime,
  formatLogTime,
  formatValueRepr,
  regRowOrder,
  shortId,
} from "./dom.js";
import { initI18n, substitute, t, tr } from "./i18n.js";
import {
  attachCancelHandlers,
  attachDeleteHandlers,
  attachRestartHandlers,
  cancelButtonHtml,
  deleteButtonHtml,
  initNewJobForm,
  restartButtonHtml,
} from "./new_job.js";
import type {
  AboutInfo,
  Campaign,
  CampaignsResponse,
  DisasmCacheEntry,
  DisasmListing,
  DutRecord,
  DutSnapshot,
  DutsResponse,
  EffectiveIsa,
  Job,
  JobLogEntry,
  JobLogsResponse,
  JobStateInfo,
  JobsResponse,
  LeaseSummary,
  SourceTag,
  State,
  VerdictKind,
  WsEvent,
} from "./types.js";

// ---------- DOM refs ----------

const statusEl = document.getElementById("status");
const jobsTbody = document.getElementById("jobs-tbody");
const campaignsTbody = document.getElementById("campaigns-tbody");
const dutsList = document.getElementById("duts-list");
const tabs = document.querySelectorAll<HTMLElement>(".tab");
const views: Record<string, HTMLElement | null> = {
  jobs: document.getElementById("view-jobs"),
  campaigns: document.getElementById("view-campaigns"),
  duts: document.getElementById("view-duts"),
  about: document.getElementById("view-about"),
};

// ---------- state ----------

// Per-job stage log buffer, capped per job so a long-lived job can't
// unbounded-grow. Keyed by full JobId.
const JOB_LOG_LIMIT = 200;
const jobLogs: Map<string, JobLogEntry[]> = new Map();
// Job ids whose detail row is currently expanded.
const expandedJobs: Set<string> = new Set();
// Cached disassembly listings keyed by job id. Either a listing or an
// `{ error }` envelope so we don't refetch when the kind has no
// disassemblable program.
const jobDisasm: Map<string, DisasmCacheEntry> = new Map();
// dut id -> job id mapping so the WS DutStateSnapshot can locate
// which job-detail panel (if any) to re-render with the new PC.
const dutForJob: Map<string, string> = new Map();
// Per-DUT latest register snapshots indexed by id, mirroring the
// /duts response shape and the snapshot WS event.
const dutSnapshots: Map<string, DutSnapshot[]> = new Map();

// Pagination state for the jobs table. The pager fetches one page
// at a time from /jobs?limit=PAGE_SIZE&offset=jobsOffset and the
// "Next/Prev" buttons advance/rewind in PAGE_SIZE chunks.
const JOBS_PAGE_SIZE = 50;
let jobsOffset = 0;
let jobsTotal = 0;

// ---------- job log ----------

function pushJobLog(jobId: string, entry: JobLogEntry): void {
  let buf = jobLogs.get(jobId);
  if (!buf) {
    buf = [];
    jobLogs.set(jobId, buf);
  }
  buf.push(entry);
  if (buf.length > JOB_LOG_LIMIT) buf.splice(0, buf.length - JOB_LOG_LIMIT);
  if (expandedJobs.has(jobId)) renderJobLog(jobId);
}

async function backfillJobLog(jobId: string): Promise<void> {
  try {
    const r = await fetch(`/jobs/${encodeURIComponent(jobId)}/logs`);
    if (!r.ok) return;
    const body = (await r.json()) as JobLogsResponse;
    const entries = (body.logs || [])
      .filter((row): row is JobLogEntry & { id?: number } => !!row.ts)
      .map<JobLogEntry>((row) => ({
        level: row.level || "info",
        message: row.message || "",
        stage: row.stage || null,
        i18n_key: row.i18n_key || null,
        i18n_args: row.i18n_args || null,
        ts: row.ts,
      }));
    const trimmed =
      entries.length > JOB_LOG_LIMIT
        ? entries.slice(entries.length - JOB_LOG_LIMIT)
        : entries;
    jobLogs.set(jobId, trimmed);
    if (expandedJobs.has(jobId)) renderJobLog(jobId);
  } catch (e) {
    console.warn("backfillJobLog", e);
  }
}

function renderJobLog(jobId: string): void {
  const host = document.querySelector<HTMLElement>(
    `tr.job-log-row[data-job="${jobId}"] .job-log`,
  );
  if (!host) return;
  const buf = jobLogs.get(jobId) || [];
  if (!buf.length) {
    host.innerHTML = `<p class="job-log-empty">${escapeHtml(t("web.jobs.no_logs", "no log events yet"))}</p>`;
    return;
  }
  host.innerHTML = buf
    .map((e) => {
      const stageLabel = e.stage ? t(`log.stage.${e.stage}`, e.stage) : null;
      const stage = stageLabel
        ? `<span class="log-stage">[${escapeHtml(stageLabel)}]</span> `
        : "";
      const levelLabel = t(`log.level.${e.level}`, e.level);
      const level = `<span class="log-level log-level-${e.level}">${escapeHtml(levelLabel)}</span>`;
      const tsStr = formatLogTime(e.ts);
      const ts = tsStr
        ? `<span class="log-ts">${escapeHtml(tsStr)}</span> `
        : "";
      const msg = escapeHtml(localizeLogMessage(e));
      return `<div class="log-line">${ts}${level} ${stage}${msg}</div>`;
    })
    .join("");
}

function localizeLogMessage(entry: JobLogEntry): string {
  if (entry.i18n_key) {
    const tmpl = t(entry.i18n_key, entry.message || entry.i18n_key);
    return substitute(tmpl, entry.i18n_args || undefined);
  }
  return entry.message || "";
}

// ---------- job detail (log + disasm) ----------

function toggleJobDetail(jobId: string): void {
  if (expandedJobs.has(jobId)) {
    expandedJobs.delete(jobId);
    void refreshJobs();
    return;
  }
  expandedJobs.add(jobId);
  void refreshJobs();
  void backfillJobLog(jobId);
  void backfillJobDisasm(jobId);
}

async function backfillJobDisasm(jobId: string): Promise<void> {
  if (jobDisasm.has(jobId)) {
    renderJobDisasm(jobId);
    return;
  }
  try {
    const r = await fetch(`/jobs/${encodeURIComponent(jobId)}/disasm`);
    if (r.status === 400) {
      const body = (await r.json().catch(() => ({}))) as {
        error?: string;
        i18n_key?: string;
        i18n_args?: Record<string, string>;
      };
      jobDisasm.set(jobId, {
        error: body.error || "unsupported",
        i18n_key: body.i18n_key,
        i18n_args: body.i18n_args,
      });
    } else if (!r.ok) {
      const body = (await r.json().catch(() => ({}))) as {
        error?: string;
        i18n_key?: string;
        i18n_args?: Record<string, string>;
      };
      jobDisasm.set(jobId, {
        error: body.error || `HTTP ${r.status}`,
        i18n_key: body.i18n_key,
        i18n_args: body.i18n_args,
      });
    } else {
      const listing = (await r.json()) as DisasmListing;
      jobDisasm.set(jobId, listing);
    }
    renderJobDisasm(jobId);
  } catch (e) {
    console.warn("backfillJobDisasm", e);
    jobDisasm.set(jobId, { error: String(e) });
    renderJobDisasm(jobId);
  }
}

function renderJobDisasm(jobId: string): void {
  const host = document.querySelector<HTMLElement>(
    `tr.job-log-row[data-job="${jobId}"] .job-disasm`,
  );
  if (!host) return;
  const cached = jobDisasm.get(jobId);
  if (!cached) {
    host.innerHTML = `<p class="job-disasm-empty">${escapeHtml(t("web.jobs.disasm_loading", "loading disassembly..."))}</p>`;
    return;
  }
  if ("error" in cached) {
    // Prefer the daemon-supplied i18n_key when present so the
    // operator sees the message in their locale. Fall back to the
    // English `error` string for cases the daemon didn't tag.
    const localized = cached.i18n_key
      ? substitute(t(cached.i18n_key, cached.error), cached.i18n_args)
      : cached.error;
    host.innerHTML = `<p class="job-disasm-empty">${escapeHtml(localized)}</p>`;
    return;
  }
  const dutId = dutForJob.get(jobId);
  const pc = dutId ? currentPcForDut(dutId) : null;
  const iterLabel =
    typeof cached.iter === "number"
      ? ` <span class="disasm-iter">${escapeHtml(t("web.jobs.disasm_iter", "iter"))} ${cached.iter}</span>`
      : "";
  const header = `<div class="disasm-header"><span class="disasm-title">${escapeHtml(t("web.jobs.disasm_title", "Disassembly"))}</span> <span class="disasm-meta">${escapeHtml(cached.name)} / ${escapeHtml(cached.isa)}</span>${iterLabel}</div>`;
  const rows = (cached.lines || [])
    .map((line) => {
      const hit = pc !== null && line.addr === pc;
      const cls = hit ? "disasm-line disasm-pc-hit" : "disasm-line";
      const addrStr = `0x${Number(line.addr).toString(16).padStart(8, "0")}`;
      const bytesStr = Array.isArray(line.bytes)
        ? line.bytes.map((b) => b.toString(16).padStart(2, "0")).join(" ")
        : "";
      return `<tr class="${cls}"><td class="disasm-addr">${escapeHtml(addrStr)}</td><td class="disasm-bytes">${escapeHtml(bytesStr)}</td><td class="disasm-mnemonic">${escapeHtml(line.mnemonic)}</td><td class="disasm-operands">${escapeHtml(line.operands)}</td></tr>`;
    })
    .join("");
  host.innerHTML = `${header}<table class="disasm-table">${rows}</table>`;
  if (pc !== null) {
    const hit = host.querySelector(".disasm-pc-hit");
    if (hit) hit.scrollIntoView({ block: "nearest" });
  }
}

function currentPcForDut(dutId: string): number | null {
  const snaps = dutSnapshots.get(dutId);
  if (!Array.isArray(snaps)) return null;
  const dut = snaps.find((s) => s.source === "dut");
  if (!dut || !dut.state || !dut.state.fields) return null;
  const repr = dut.state.fields.pc;
  if (repr && repr.type === "u64" && typeof repr.value === "number") {
    return repr.value;
  }
  return null;
}

// ---------- jobs view ----------

function jobStateLabel(
  state: JobStateInfo | null | undefined,
): [string, string] {
  if (!state) return ["", "-"];
  const tag = state.state;
  const localizedState = t(`tui.job_state.${tag}`, tag);
  if (tag === "done") {
    const detail = typeof state.detail === "object" ? state.detail : null;
    const v: VerdictKind = (detail && detail.kind) || "pass";
    const localizedVerdict = t(`tui.verdict.${v}`, v);
    return [`state-done verdict-${v}`, `${localizedState}/${localizedVerdict}`];
  }
  if (tag === "failed") {
    const reason = typeof state.detail === "string" ? state.detail : "";
    return ["state-failed", `${localizedState}: ${reason}`];
  }
  return [`state-${tag}`, localizedState];
}

function jobKindLabel(kindTag: string): string {
  return t(`tui.job_kind.${kindTag}`, kindTag);
}

function renderJobs(jobs: Job[]): void {
  if (!jobsTbody) return;
  if (!jobs.length) {
    jobsTbody.innerHTML = `<tr class="empty"><td colspan="5">${escapeHtml(t("tui.empty.no_jobs", "no jobs"))}</td></tr>`;
    return;
  }
  // Snapshot scroll positions of every currently-expanded detail
  // panel BEFORE we rebuild the tbody. The 5s polling refresh
  // replaces the whole DOM subtree wholesale, which would otherwise
  // reset .job-disasm / .job-log scrollTop to 0 mid-read. Restored
  // below after innerHTML lands.
  const scrolls = captureDetailScrolls();
  jobsTbody.innerHTML = jobs
    .map((j) => {
      dutForJob.set(j.id, j.dut);
      const [cls, label] = jobStateLabel(j.state);
      const kindTag = (j.kind && j.kind.kind) || "?";
      const kindLabel = jobKindLabel(kindTag);
      const expanded = expandedJobs.has(j.id);
      const arrowClass = expanded ? "icon-row-open" : "icon-row-closed";
      const arrow = `<i class="lucide ${arrowClass}" aria-hidden="true"></i>`;
      const stateTag = j.state ? j.state.state : null;
      const cancellable = stateTag === "queued" || stateTag === "running";
      // Restart only after a terminal transition. Excluding queued/running
      // prevents accidentally double-booking a DUT mid-flight.
      const restartable =
        stateTag === "done" ||
        stateTag === "failed" ||
        stateTag === "cancelled" ||
        stateTag === "dead";
      // Per-row delete: only terminal rows. The route refuses
      // queued/running anyway, but hiding the button prevents the
      // operator from even trying.
      const deletable = restartable;
      const deleteBtn = deletable ? deleteButtonHtml(j.id) : "";
      const primaryAction = cancellable
        ? cancelButtonHtml(j.id)
        : restartable
          ? restartButtonHtml(j.id)
          : "";
      const actionsCell = primaryAction + deleteBtn;
      const head = `
        <tr class="job-row" data-job="${escapeHtml(j.id)}">
          <td class="id"><span class="job-toggle">${arrow}</span> ${shortId(j.id)}</td>
          <td>${escapeHtml(j.dut)}</td>
          <td>${escapeHtml(kindLabel)}</td>
          <td class="state-cell ${cls}" title="${escapeHtml(label)}"><span class="state-text">${escapeHtml(label)}</span></td>
          <td>${escapeHtml(fmtTime(j.created_at))}</td>
          <td class="row-actions">${actionsCell}</td>
        </tr>`;
      const detail = expanded
        ? `<tr class="job-log-row" data-job="${escapeHtml(j.id)}"><td colspan="6"><div class="job-log"></div><div class="job-disasm"></div></td></tr>`
        : "";
      return head + detail;
    })
    .join("");

  jobsTbody.querySelectorAll<HTMLElement>("tr.job-row").forEach((row) => {
    row.addEventListener("click", (e) => {
      // The cancel and restart buttons live inside the row but
      // should not toggle the detail panel. Ignore clicks that
      // originated on either action button.
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.closest("[data-cancel-job]") ||
          target.closest("[data-restart-job]") ||
          target.closest("[data-delete-job]"))
      )
        return;
      const id = row.dataset.job;
      if (id) toggleJobDetail(id);
    });
  });
  attachCancelHandlers(jobsTbody, () => void refreshJobs());
  attachRestartHandlers(jobsTbody, () => void refreshJobs());
  attachDeleteHandlers(jobsTbody, () => void refreshJobs());
  expandedJobs.forEach((id) => {
    renderJobLog(id);
    renderJobDisasm(id);
  });
  // Restore scroll positions captured pre-render so a poll refresh
  // doesn't yank the user back to the top of a long disasm listing.
  restoreDetailScrolls(scrolls);
}

interface DetailScrolls {
  log: Map<string, number>;
  disasm: Map<string, number>;
}

function captureDetailScrolls(): DetailScrolls {
  const out: DetailScrolls = { log: new Map(), disasm: new Map() };
  if (!jobsTbody) return out;
  jobsTbody.querySelectorAll<HTMLElement>("tr.job-log-row").forEach((row) => {
    const id = row.dataset.job;
    if (!id) return;
    const logEl = row.querySelector<HTMLElement>(".job-log");
    const disasmEl = row.querySelector<HTMLElement>(".job-disasm");
    if (logEl && logEl.scrollTop > 0) out.log.set(id, logEl.scrollTop);
    if (disasmEl && disasmEl.scrollTop > 0)
      out.disasm.set(id, disasmEl.scrollTop);
  });
  return out;
}

function restoreDetailScrolls(s: DetailScrolls): void {
  if (!jobsTbody) return;
  jobsTbody.querySelectorAll<HTMLElement>("tr.job-log-row").forEach((row) => {
    const id = row.dataset.job;
    if (!id) return;
    const logEl = row.querySelector<HTMLElement>(".job-log");
    const disasmEl = row.querySelector<HTMLElement>(".job-disasm");
    const logY = s.log.get(id);
    const disasmY = s.disasm.get(id);
    if (logEl && logY !== undefined) logEl.scrollTop = logY;
    if (disasmEl && disasmY !== undefined) disasmEl.scrollTop = disasmY;
  });
}

async function refreshJobs(): Promise<void> {
  try {
    const url = `/jobs?limit=${JOBS_PAGE_SIZE}&offset=${jobsOffset}`;
    const r = await fetch(url);
    if (!r.ok) throw new Error(String(r.status));
    const body = (await r.json()) as JobsResponse;
    jobsTotal = body.total || 0;
    // If a prune deleted enough rows that our offset now points past
    // the end, rewind to a valid page so the table isn't empty when
    // there's still data to show.
    if (jobsOffset > 0 && jobsOffset >= jobsTotal) {
      const lastPageStart =
        Math.max(0, Math.floor((jobsTotal - 1) / JOBS_PAGE_SIZE)) *
        JOBS_PAGE_SIZE;
      if (lastPageStart !== jobsOffset) {
        jobsOffset = lastPageStart;
        return refreshJobs();
      }
    }
    renderJobs(body.jobs || []);
    renderJobsPager();
  } catch (e) {
    console.warn("refreshJobs", e);
  }
}

function renderJobsPager(): void {
  const info = document.getElementById("jobs-page-info");
  const prev = document.getElementById("jobs-prev") as HTMLButtonElement | null;
  const next = document.getElementById("jobs-next") as HTMLButtonElement | null;
  if (!info || !prev || !next) return;
  const pages = Math.max(1, Math.ceil(jobsTotal / JOBS_PAGE_SIZE));
  const page = Math.floor(jobsOffset / JOBS_PAGE_SIZE) + 1;
  info.textContent = substitute(
    t("web.jobs.page", "page {page} of {pages} ({total} total)"),
    { page: String(page), pages: String(pages), total: String(jobsTotal) },
  );
  prev.disabled = jobsOffset === 0;
  next.disabled = jobsOffset + JOBS_PAGE_SIZE >= jobsTotal;
}

// ---------- campaigns view ----------

function renderCampaigns(campaigns: Campaign[]): void {
  if (!campaignsTbody) return;
  if (!campaigns.length) {
    campaignsTbody.innerHTML = `<tr class="empty"><td colspan="5">${escapeHtml(t("tui.empty.no_campaigns", "no campaigns"))}</td></tr>`;
    return;
  }
  campaignsTbody.innerHTML = campaigns
    .map((c) => {
      const state = (c.state && c.state.state) || "?";
      const tpl = (c.template && c.template.kind) || "?";
      return `
        <tr>
          <td class="id">${shortId(c.id)}</td>
          <td>${escapeHtml(c.dut)}</td>
          <td>${escapeHtml(tpl)}</td>
          <td class="campaign-state-${state}">${escapeHtml(state)}</td>
          <td>${escapeHtml(c.chip_serial || "-")}</td>
        </tr>`;
    })
    .join("");
}

async function refreshCampaigns(): Promise<void> {
  try {
    const r = await fetch("/campaigns");
    if (!r.ok) throw new Error(String(r.status));
    const body = (await r.json()) as CampaignsResponse;
    renderCampaigns(body.campaigns || []);
  } catch (e) {
    console.warn("refreshCampaigns", e);
  }
}

// ---------- duts view ----------

// RV ABI names indexed by hardware register number. State uses `xN`
// keys per Heimdall convention. The UI layers the ABI label on top.
// `fp` matches OpenOCD's gpr_names[] (riscv-013.c).
const RV_ABI_NAMES = [
  "zero",
  "ra",
  "sp",
  "gp",
  "tp",
  "t0",
  "t1",
  "t2",
  "fp",
  "s1",
  "a0",
  "a1",
  "a2",
  "a3",
  "a4",
  "a5",
  "a6",
  "a7",
  "s2",
  "s3",
  "s4",
  "s5",
  "s6",
  "s7",
  "s8",
  "s9",
  "s10",
  "s11",
  "t3",
  "t4",
  "t5",
  "t6",
] as const;

function abiNameForHwReg(name: string): string | null {
  const m = /^x(\d+)$/.exec(name);
  if (!m || m[1] === undefined) return null;
  const n = Number(m[1]);
  if (Number.isInteger(n) && n >= 0 && n < RV_ABI_NAMES.length) {
    return RV_ABI_NAMES[n] || null;
  }
  return null;
}

function renderState(state: State | null | undefined): string {
  if (!state || !state.fields) return "";
  const entries = Object.entries(state.fields).sort(([a], [b]) => {
    const [ka, na, sa] = regRowOrder(a);
    const [kb, nb, sb] = regRowOrder(b);
    if (ka !== kb) return ka - kb;
    if (na !== nb) return na - nb;
    return sa < sb ? -1 : sa > sb ? 1 : 0;
  });
  const rows = entries.map(([name, repr]) => {
    const value = formatValueRepr(repr);
    const abi = abiNameForHwReg(name);
    const label = abi
      ? `<span class="reg-abi">${escapeHtml(abi)}</span><span class="reg-hw">${escapeHtml(name)}</span>`
      : escapeHtml(name);
    return `<tr><td class="reg-name">${label}</td><td class="reg-val">${escapeHtml(value)}</td></tr>`;
  });
  if (!rows.length) return "";
  return `<table class="dut-regs">${rows.join("")}</table>`;
}

/// Render the per-DUT ISA strip: XLEN, source badge, one chip per
/// enabled extension. Returns "" when the daemon couldn't resolve
/// an effective ISA so we don't lie with a base-I default the
/// silicon may not actually run.
function renderIsa(isa: EffectiveIsa | undefined): string {
  if (!isa) {
    return `<div class="dut-isa dut-isa-unknown"><span class="dut-isa-label">${escapeHtml(t("web.duts.isa", "ISA"))}</span> <span class="dut-isa-empty">${escapeHtml(t("web.duts.isa_unknown", "unknown"))}</span></div>`;
  }
  const xlen = `RV${isa.xlen}`;
  const sourceLabel = t(`web.duts.isa_source.${isa.source}`, isa.source);
  const sourceClass = `dut-isa-source-${isa.source}`;
  const chips = (isa.extensions || [])
    .map(
      (e) => `<span class="dut-isa-ext">${escapeHtml(e.toUpperCase())}</span>`,
    )
    .join("");
  return `
    <div class="dut-isa">
      <span class="dut-isa-label">${escapeHtml(t("web.duts.isa", "ISA"))}</span>
      <span class="dut-isa-xlen">${escapeHtml(xlen)}</span>
      <span class="dut-isa-chips">${chips}</span>
      <span class="dut-isa-source ${sourceClass}" title="${escapeHtml(sourceLabel)}">${escapeHtml(sourceLabel)}</span>
    </div>`;
}

function renderDuts(
  duts: DutRecord[],
  leases: LeaseSummary[] | undefined,
): void {
  if (!dutsList) return;
  if (!duts.length) {
    dutsList.innerHTML = `<p class="empty-block">${escapeHtml(t("web.duts.no_duts", "no DUTs configured"))}</p>`;
    return;
  }
  const jtagPrefix = t("web.duts.jtag_prefix", "jtag:");
  const showNetlist = t("web.duts.show_netlist", "Show netlist");
  const statusLabels: Record<string, string> = {
    connected: t("common.status.connected", "connected"),
    disconnected: t("common.status.disconnected", "disconnected"),
    idle: t("common.status.idle", "idle"),
    "in-use": t("common.status.in_use", "in use"),
  };
  const sourceLabels: Record<SourceTag, string> = {
    dut: t("web.duts.source_dut", "dut"),
    golden: t("web.duts.source_golden", "golden"),
  };
  const leaseByDut: Map<string, string> = new Map();
  for (const lease of leases || []) {
    if (lease && lease.dut && lease.holder)
      leaseByDut.set(lease.dut, lease.holder);
  }
  for (const d of duts) {
    if (Array.isArray(d.snapshots) && d.snapshots.length) {
      dutSnapshots.set(d.id, d.snapshots);
    }
  }
  dutsList.innerHTML = duts
    .map((d) => {
      const id = escapeHtml(d.id);
      const kind = escapeHtml(d.kind);
      const serial = d.chip_serial ? escapeHtml(d.chip_serial) : "-";
      const jtag = d.jtag && d.jtag.driver ? escapeHtml(d.jtag.driver) : "-";
      const holder = leaseByDut.get(d.id);
      let statusClass: string;
      if (holder) {
        statusClass = "in-use";
      } else {
        const rawStatus = d.connection_status || "unknown";
        statusClass = rawStatus === "unknown" ? "idle" : rawStatus;
      }
      const statusLabel = statusLabels[statusClass] || statusClass;
      const isaBlock = renderIsa(d.effective_isa);
      const hasNetlist = !!d.netlist;
      const netlistBlock = hasNetlist
        ? `
        <details class="netlist-panel">
          <summary>${escapeHtml(showNetlist)}</summary>
          <div class="netlist-svg">
            <img src="/duts/${encodeURIComponent(d.id)}/netlist.svg" alt="netlist for ${id}" />
          </div>
        </details>`
        : "";
      const snapshots = dutSnapshots.get(d.id) || [];
      const stateBlocks = snapshots
        .map((s) => {
          const sourceLabel = sourceLabels[s.source] || s.source;
          const tsLabel = s.ts ? formatLogTime(s.ts) : "";
          const tsBlock = tsLabel
            ? `<span class="reg-ts">${escapeHtml(tsLabel)}</span>`
            : "";
          return `
          <details class="dut-state-panel" open>
            <summary><span class="reg-source">${escapeHtml(sourceLabel)}</span> ${tsBlock}</summary>
            ${renderState(s.state)}
          </details>`;
        })
        .join("");
      return `
        <div class="dut-card" data-dut="${id}">
          <div class="dut-header">
            <span class="dut-id">${id}</span>
            <span class="dut-kind">${kind}</span>
            <span class="dut-serial">${serial}</span>
            <span class="dut-jtag">${escapeHtml(jtagPrefix)} ${jtag}</span>
            <span class="dut-status dut-status-${statusClass}">${escapeHtml(statusLabel)}</span>
          </div>
          ${isaBlock}
          ${stateBlocks}
          ${netlistBlock}
        </div>`;
    })
    .join("");
}

async function refreshDuts(): Promise<void> {
  try {
    const r = await fetch("/duts");
    if (!r.ok) throw new Error(String(r.status));
    const body = (await r.json()) as DutsResponse;
    renderDuts(body.duts || [], body.leases);
  } catch (e) {
    console.warn("refreshDuts", e);
  }
}

// ---------- about ----------

// Cached so we only hit /about once per session. The data is
// rooted in the daemon's compile-time cfg, so a refresh would
// just return the same struct.
let aboutCache: AboutInfo | null = null;
let aboutInFlight: Promise<void> | null = null;

async function loadAbout(): Promise<void> {
  if (aboutCache) {
    renderAbout(aboutCache);
    return;
  }
  if (aboutInFlight) return aboutInFlight;
  aboutInFlight = (async () => {
    try {
      const r = await fetch("/about");
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const body = (await r.json()) as AboutInfo;
      aboutCache = body;
      renderAbout(body);
    } catch (e) {
      const el = document.getElementById("about-version");
      if (el) {
        el.textContent = substitute(
          t("web.about.load_failed", "load failed: {error}"),
          { error: String(e) },
        );
      }
    } finally {
      aboutInFlight = null;
    }
  })();
  return aboutInFlight;
}

function renderAbout(info: AboutInfo): void {
  const v = document.getElementById("about-version");
  const b = document.getElementById("about-build-profile");
  const f = document.getElementById("about-features");
  if (v) v.textContent = info.version;
  if (b) {
    const profileLabel = t(
      `web.about.profile.${info.build_profile}`,
      info.build_profile,
    );
    b.textContent = profileLabel;
  }
  if (f) {
    const enabled = Object.entries(info.features)
      .filter(([, on]) => on)
      .map(([name]) => name)
      .sort();
    f.textContent = enabled.length
      ? enabled.join(", ")
      : t("web.about.no_features", "none");
  }
}

// ---------- view tabs ----------

function activate(view: string): void {
  tabs.forEach((tab) =>
    tab.classList.toggle("active", tab.dataset.view === view),
  );
  Object.entries(views).forEach(([k, el]) => {
    if (el) el.classList.toggle("hidden", k !== view);
  });
  // About is fetched lazily on first activation rather than on
  // boot: it never changes within a daemon's lifetime so a single
  // load is enough, and front-loading it would just slow the
  // initial paint.
  if (view === "about") void loadAbout();
}

tabs.forEach((tab) => {
  tab.addEventListener("click", () => {
    const v = tab.dataset.view;
    if (v) activate(v);
  });
});
document.addEventListener("keydown", (e) => {
  if (e.key === "1") activate("jobs");
  if (e.key === "2") activate("campaigns");
  if (e.key === "3") activate("duts");
  if (e.key === "4") activate("about");
});

// ---------- websocket ----------

function openSocket(): void {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const url = `${proto}://${location.host}/events`;
  let ws: WebSocket;
  try {
    ws = new WebSocket(url);
  } catch (_) {
    if (statusEl) {
      statusEl.textContent = t("common.status.disconnected", "disconnected");
    }
    return;
  }
  ws.onopen = () => {
    if (statusEl) {
      statusEl.textContent = t("common.status.connected", "connected");
    }
  };
  ws.onclose = () => {
    if (statusEl) {
      statusEl.textContent = t(
        "tui.disconnected_short",
        "disconnected, retrying...",
      );
    }
    setTimeout(openSocket, 2000);
  };
  ws.onerror = () => {
    if (statusEl) {
      statusEl.textContent = t("common.status.disconnected", "disconnected");
    }
  };
  ws.onmessage = (e: MessageEvent<string>) => {
    try {
      const ev = JSON.parse(e.data) as WsEvent;
      const kind = ev.kind || "?";
      if (statusEl) statusEl.textContent = tr("tui.event_label", { kind });
      if (kind === "job-log") {
        const log = ev as { ts?: string; job?: string } & Partial<JobLogEntry>;
        if (!log.ts || !log.job) {
          console.warn("dropping job-log frame missing job or ts", ev);
          return;
        }
        pushJobLog(log.job, {
          level: log.level || "info",
          message: log.message || "",
          stage: log.stage || null,
          i18n_key: log.i18n_key || null,
          i18n_args: log.i18n_args || null,
          ts: log.ts,
        });
        return;
      }
      if (kind === "dut-state-snapshot") {
        const snap = ev as {
          dut?: string;
          ts?: string;
          source?: SourceTag;
          job?: string;
          state?: State;
        };
        if (!snap.dut || !snap.ts || !snap.source || !snap.state) {
          console.warn("dropping dut-state-snapshot frame missing fields", ev);
          return;
        }
        const list = dutSnapshots.get(snap.dut) || [];
        const next = list.filter((s) => s.source !== snap.source);
        next.push({
          source: snap.source,
          ts: snap.ts,
          job: snap.job ?? null,
          state: snap.state,
        });
        next.sort((a, b) => (a.source === "dut" ? -1 : 1));
        dutSnapshots.set(snap.dut, next);
        void refreshDuts();
        if (snap.source === "dut") {
          expandedJobs.forEach((jobId) => {
            if (dutForJob.get(jobId) !== snap.dut) return;
            const cached = jobDisasm.get(jobId);
            if (
              cached &&
              !("error" in cached) &&
              typeof cached.iter === "number"
            ) {
              jobDisasm.delete(jobId);
              void backfillJobDisasm(jobId);
            } else {
              renderJobDisasm(jobId);
            }
          });
        }
        return;
      }
      if (kind.startsWith("job-") || kind.startsWith("lease-"))
        void refreshJobs();
      if (kind.startsWith("campaign-")) void refreshCampaigns();
      if (kind.startsWith("lease-")) void refreshDuts();
    } catch (_) {
      // ignore parse errors
    }
  };
}

// ---------- boot ----------

void initI18n().then(() => {
  void refreshJobs();
  void refreshCampaigns();
  void refreshDuts();
  initNewJobForm(() => void refreshJobs());
  wireJobsPager();
  wirePruneDialog();
  openSocket();
  setInterval(() => void refreshJobs(), 5000);
  setInterval(() => void refreshCampaigns(), 5000);
  // DUTs change only on daemon restart. Refresh less aggressively.
  setInterval(() => void refreshDuts(), 30000);
});

function wireJobsPager(): void {
  const prev = document.getElementById("jobs-prev");
  const next = document.getElementById("jobs-next");
  if (prev) {
    prev.addEventListener("click", () => {
      jobsOffset = Math.max(0, jobsOffset - JOBS_PAGE_SIZE);
      void refreshJobs();
    });
  }
  if (next) {
    next.addEventListener("click", () => {
      if (jobsOffset + JOBS_PAGE_SIZE < jobsTotal) {
        jobsOffset += JOBS_PAGE_SIZE;
        void refreshJobs();
      }
    });
  }
}

function wirePruneDialog(): void {
  const open = document.getElementById("jobs-prune-open");
  const dialog = document.getElementById(
    "jobs-prune-dialog",
  ) as HTMLDialogElement | null;
  const cancel = document.getElementById("prune-cancel");
  const confirm = document.getElementById("prune-confirm");
  const daysInput = document.getElementById(
    "prune-days",
  ) as HTMLInputElement | null;
  if (!open || !dialog || !cancel || !confirm || !daysInput) return;
  open.addEventListener("click", () => dialog.showModal());
  cancel.addEventListener("click", () => dialog.close());
  confirm.addEventListener("click", async () => {
    const days = Math.max(0, parseInt(daysInput.value, 10) || 0);
    const cutoff = new Date(Date.now() - days * 24 * 60 * 60 * 1000);
    try {
      const r = await fetch(
        `/jobs?before=${encodeURIComponent(cutoff.toISOString())}`,
        { method: "DELETE" },
      );
      if (!r.ok) {
        const body = (await r.json().catch(() => ({}))) as { error?: string };
        const msg = body.error || `HTTP ${r.status}`;
        alert(
          substitute(t("web.jobs.prune_failed", "prune failed: {error}"), {
            error: msg,
          }),
        );
        return;
      }
      const body = (await r.json()) as { deleted: number };
      alert(
        substitute(t("web.jobs.prune_done", "Pruned {count} job(s)."), {
          count: String(body.deleted),
        }),
      );
      dialog.close();
      jobsOffset = 0;
      void refreshJobs();
    } catch (e) {
      alert(
        substitute(t("web.jobs.prune_failed", "prune failed: {error}"), {
          error: String(e),
        }),
      );
    }
  });
}
