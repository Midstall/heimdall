// New-job form: collects the operator's input, builds the right
// JobKind payload, base64-encodes any uploaded files, and POSTs to
// /jobs. The form's DOM scaffold lives in index.html. This module
// just wires the kind-dropdown, the submit, and per-variant payload
// assembly.

import { escapeHtml } from "./dom.js";
import { substitute, t } from "./i18n.js";
import type { JobKind } from "./types.js";

/// Read a File as a base64 string. The daemon's JobKind variants
/// carry bitstreams/ELFs as base64 (`elf_b64`, `bitstream_b64`) on
/// the wire. The upload widgets here pump bytes through this helper
/// before submission.
async function fileToBase64(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const bytes = new Uint8Array(buf);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

/// Read a File as UTF-8 text. Used for the Aegis descriptor JSON
/// which the daemon expects as a string field, not base64.
async function fileToText(file: File): Promise<string> {
  return await file.text();
}

/// Parse `name=true` / `name=false` lines (one per line) into a
/// flat record. Blank lines and lines without `=` are skipped.
/// Used for the Aegis vector form's inputs/expected_outputs maps.
function parseBoolMap(text: string): Record<string, boolean> {
  const out: Record<string, boolean> = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const name = line.slice(0, eq).trim();
    const value = line
      .slice(eq + 1)
      .trim()
      .toLowerCase();
    if (!name) continue;
    out[name] = value === "true" || value === "1" || value === "yes";
  }
  return out;
}

function getFormElement(
  form: HTMLFormElement,
  name: string,
): HTMLElement | null {
  return form.elements.namedItem(name) as HTMLElement | null;
}

function getStringField(form: HTMLFormElement, name: string): string {
  const el = getFormElement(form, name);
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    return el.value;
  }
  if (el instanceof HTMLSelectElement) return el.value;
  return "";
}

function getNumberField(
  form: HTMLFormElement,
  name: string,
  fallback: number,
): number {
  // `Number("")` is 0, not NaN, so a blank input would silently
  // submit zero unless we treat the empty string as "use the
  // fallback default" explicitly. Also catches whitespace-only
  // entries that the browser's `min=` constraint doesn't notice.
  const raw = getStringField(form, name).trim();
  if (raw.length === 0) return fallback;
  const n = Number(raw);
  return Number.isFinite(n) ? n : fallback;
}

function getCheckboxField(form: HTMLFormElement, name: string): boolean {
  const el = getFormElement(form, name);
  return el instanceof HTMLInputElement ? el.checked : false;
}

function getFileField(form: HTMLFormElement, name: string): File | null {
  const el = getFormElement(form, name);
  return el instanceof HTMLInputElement && el.files && el.files.length > 0
    ? (el.files[0] ?? null)
    : null;
}

/// Build the JobKind payload from the form's current state. Returns
/// either the JobKind to POST, or an error message the caller
/// surfaces. Doing it as a discriminated function keeps the
/// variant-specific JSON shapes localized and grep-friendly.
async function buildJobKind(
  kind: string,
  form: HTMLFormElement,
): Promise<{ ok: true; value: JobKind } | { ok: false; error: string }> {
  switch (kind) {
    case "mock-hello":
      return { ok: true, value: { kind: "mock-hello" } as JobKind };
    case "fuzz": {
      const generator = getStringField(form, "generator") || "raw-asm";
      return {
        ok: true,
        value: {
          kind: "fuzz",
          iterations: getNumberField(form, "iterations", 1),
          seed: getNumberField(form, "seed", 0),
          insn_count: getNumberField(form, "insn_count", 16),
          cycles: getNumberField(form, "cycles", 1000),
          strict_coverage: getCheckboxField(form, "strict_coverage"),
          generator,
        } as JobKind,
      };
    }
    case "boot-river-elf": {
      const file = getFileField(form, "elf_file");
      if (!file) {
        return {
          ok: false,
          error: t("web.new_job.file_required", "file required"),
        };
      }
      const elf_b64 = await fileToBase64(file);
      return {
        ok: true,
        value: {
          kind: "boot-river-elf",
          elf_b64,
          cycles: getNumberField(form, "cycles", 1000),
        } as JobKind,
      };
    }
    case "load-aegis-bitstream": {
      const descFile = getFileField(form, "descriptor_file");
      const bitFile = getFileField(form, "bitstream_file");
      if (!descFile || !bitFile) {
        return {
          ok: false,
          error: t("web.new_job.file_required", "file required"),
        };
      }
      const descriptor_json = await fileToText(descFile);
      const bitstream_b64 = await fileToBase64(bitFile);
      return {
        ok: true,
        value: {
          kind: "load-aegis-bitstream",
          descriptor_json,
          bitstream_b64,
        } as JobKind,
      };
    }
    case "run-aegis-vector": {
      const descFile = getFileField(form, "descriptor_file_v");
      const bitFile = getFileField(form, "bitstream_file_v");
      if (!descFile || !bitFile) {
        return {
          ok: false,
          error: t("web.new_job.file_required", "file required"),
        };
      }
      const descriptor_json = await fileToText(descFile);
      const bitstream_b64 = await fileToBase64(bitFile);
      const inputs = parseBoolMap(getStringField(form, "inputs"));
      const expected_outputs = parseBoolMap(
        getStringField(form, "expected_outputs"),
      );
      return {
        ok: true,
        value: {
          kind: "run-aegis-vector",
          descriptor_json,
          bitstream_b64,
          inputs,
          expected_outputs,
          settle_cycles: getNumberField(form, "settle_cycles", 0),
        } as JobKind,
      };
    }
    case "named": {
      const name = getStringField(form, "named_name").trim();
      const raw = getStringField(form, "named_payload").trim();
      let payload: unknown = null;
      if (raw.length > 0) {
        try {
          payload = JSON.parse(raw);
        } catch (_) {
          return {
            ok: false,
            error: t("web.new_job.invalid_json", "payload is not valid JSON"),
          };
        }
      }
      return { ok: true, value: { kind: "named", name, payload } as JobKind };
    }
    default:
      return { ok: false, error: `unknown kind: ${kind}` };
  }
}

interface DutSummary {
  id: string;
}

async function fetchDuts(): Promise<DutSummary[]> {
  try {
    const r = await fetch("/duts");
    if (!r.ok) return [];
    const body = (await r.json()) as { duts?: DutSummary[] };
    return body.duts || [];
  } catch (_) {
    return [];
  }
}

/// Toggle the variant-specific fieldsets based on the kind dropdown.
///
/// Visibility alone isn't enough: any `required` input under a
/// hidden fieldset still participates in `form.checkValidity()`,
/// and the browser then logs a console warning per field because
/// it can't focus a display:none control to show the error
/// bubble. We `disabled` the inputs inside the inactive fieldsets
/// (skipped by both validation and FormData) and clear `disabled`
/// on the active one. Side bonus: stray inputs from other
/// variants never sneak into the submitted payload.
function syncKindFieldsets(form: HTMLFormElement, kind: string): void {
  form.querySelectorAll<HTMLElement>(".nj-kind").forEach((el) => {
    el.classList.add("hidden");
    el.querySelectorAll<
      HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
    >("input, textarea, select").forEach((ctrl) => {
      ctrl.disabled = true;
    });
  });
  const active = form.querySelector<HTMLElement>(`.nj-kind-${kind}`);
  if (active) {
    active.classList.remove("hidden");
    active
      .querySelectorAll<
        HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
      >("input, textarea, select")
      .forEach((ctrl) => {
        ctrl.disabled = false;
      });
  }
}

function setError(message: string): void {
  const el = document.getElementById("nj-error");
  if (el) el.textContent = message;
}

/// Wire up the New Job button, the form's kind-dropdown listener,
/// the cancel button, and the submit handler. Called once during
/// app boot. Idempotent if called more than once (re-registration
/// just replaces the existing listeners via `cloneNode` reset).
export function initNewJobForm(onSubmitted: () => void): void {
  const openBtn = document.getElementById("open-new-job");
  const form = document.getElementById(
    "new-job-form",
  ) as HTMLFormElement | null;
  const cancelBtn = document.getElementById("nj-cancel");
  const dutSelect = document.getElementById(
    "nj-dut",
  ) as HTMLSelectElement | null;
  const kindSelect = document.getElementById(
    "nj-kind",
  ) as HTMLSelectElement | null;
  if (!openBtn || !form || !cancelBtn || !dutSelect || !kindSelect) return;

  openBtn.addEventListener("click", async () => {
    setError("");
    const duts = await fetchDuts();
    dutSelect.innerHTML = duts
      .map(
        (d) =>
          `<option value="${escapeHtml(d.id)}">${escapeHtml(d.id)}</option>`,
      )
      .join("");
    if (duts.length === 0) {
      setError(t("web.new_job.no_duts", "no DUTs configured"));
    }
    form.classList.remove("hidden");
    syncKindFieldsets(form, kindSelect.value || "mock-hello");
  });

  cancelBtn.addEventListener("click", () => {
    form.classList.add("hidden");
    setError("");
  });

  kindSelect.addEventListener("change", () => {
    syncKindFieldsets(form, kindSelect.value);
  });

  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    setError("");
    // Native validation first: respects `min` / `required` on
    // every input inside the currently-visible fieldset (hidden
    // fieldsets are skipped by checkValidity, so the variant
    // dropdown effectively scopes which constraints apply). The
    // `reportValidity` call pops the browser's native tooltip on
    // the offending field, which is more discoverable than the
    // shared error span at the bottom of the form.
    if (!form.checkValidity()) {
      form.reportValidity();
      return;
    }
    const kind = kindSelect.value || "mock-hello";
    const built = await buildJobKind(kind, form);
    if (!built.ok) {
      setError(built.error);
      return;
    }
    const body = {
      dut: dutSelect.value,
      kind: built.value,
    };
    try {
      const r = await fetch("/jobs", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!r.ok) {
        const errBody = await r.json().catch(() => ({}));
        const msg = (errBody as { error?: string }).error || `HTTP ${r.status}`;
        setError(
          substitute(t("web.new_job.submit_failed", "submit failed: {error}"), {
            error: msg,
          }),
        );
        return;
      }
      form.classList.add("hidden");
      // Reset most fields but keep the dut + kind so submitting
      // batches of the same shape is fast.
      form
        .querySelectorAll<
          HTMLInputElement | HTMLTextAreaElement
        >("input:not([type=hidden]):not(#nj-dut), textarea")
        .forEach((el) => {
          if (el instanceof HTMLInputElement && el.type === "checkbox") {
            el.checked = false;
          } else if (el instanceof HTMLInputElement && el.type === "file") {
            el.value = "";
          } else {
            // skip. Defaults on numbers/text are useful to preserve
          }
        });
      onSubmitted();
    } catch (e) {
      setError(
        substitute(t("web.new_job.submit_failed", "submit failed: {error}"), {
          error: String(e),
        }),
      );
    }
  });
}

/// Build the inline cancel-button HTML for a queued job row. Plain
/// HTML so the existing renderJobs path can splice it into the
/// last column via string concatenation. The actual click handler
/// is wired by `attachCancelHandlers` after the row is in the DOM.
/// Mirrors `restartButtonHtml`: the visible glyph is a Lucide `x`
/// icon and the accessible name lives on `aria-label` / `title`.
export function cancelButtonHtml(jobId: string): string {
  const label = t("web.jobs.cancel", "cancel");
  return `<button type="button" class="btn-cancel" data-cancel-job="${escapeHtml(jobId)}" title="${escapeHtml(label)}" aria-label="${escapeHtml(label)}"><i class="lucide icon-cancel" aria-hidden="true"></i></button>`;
}

/// Build the inline restart-button HTML for a terminal job row.
/// Wired by `attachRestartHandlers`. On success the new job appears
/// at the top of the table on the next refresh. The glyph is a
/// Lucide `rotate-cw` icon, served from a build-time-baked icon
/// font. The `aria-label` carries the accessible name for screen
/// readers since the rendered glyph is purely decorative.
export function restartButtonHtml(jobId: string): string {
  const label = t("web.jobs.restart", "restart");
  return `<button type="button" class="btn-restart" data-restart-job="${escapeHtml(jobId)}" title="${escapeHtml(label)}" aria-label="${escapeHtml(label)}"><i class="lucide icon-restart" aria-hidden="true"></i></button>`;
}

/// Wire click listeners on every `[data-cancel-job]` button under
/// the given root. Calls `onCancelled` after a successful cancel so
/// the jobs view re-renders.
export function attachCancelHandlers(
  root: HTMLElement,
  onCancelled: () => void,
): void {
  root
    .querySelectorAll<HTMLButtonElement>("[data-cancel-job]")
    .forEach((btn) => {
      btn.addEventListener("click", async (e) => {
        e.stopPropagation();
        const id = btn.dataset.cancelJob;
        if (!id) return;
        btn.disabled = true;
        try {
          const r = await fetch(`/jobs/${encodeURIComponent(id)}/cancel`, {
            method: "POST",
          });
          if (!r.ok) {
            const body = (await r.json().catch(() => ({}))) as {
              error?: string;
            };
            const msg = body.error || `HTTP ${r.status}`;
            alert(
              substitute(
                t("web.jobs.cancel_failed", "cancel failed: {error}"),
                {
                  error: msg,
                },
              ),
            );
            return;
          }
          onCancelled();
        } catch (e) {
          alert(
            substitute(t("web.jobs.cancel_failed", "cancel failed: {error}"), {
              error: String(e),
            }),
          );
        } finally {
          btn.disabled = false;
        }
      });
    });
}

/// Mirror of `attachCancelHandlers` for the restart button.
/// `onRestarted` re-runs the jobs view so the new row shows up
/// immediately rather than waiting for the next websocket tick.
export function attachRestartHandlers(
  root: HTMLElement,
  onRestarted: () => void,
): void {
  root
    .querySelectorAll<HTMLButtonElement>("[data-restart-job]")
    .forEach((btn) => {
      btn.addEventListener("click", async (e) => {
        e.stopPropagation();
        const id = btn.dataset.restartJob;
        if (!id) return;
        btn.disabled = true;
        try {
          const r = await fetch(`/jobs/${encodeURIComponent(id)}/restart`, {
            method: "POST",
          });
          if (!r.ok) {
            const body = (await r.json().catch(() => ({}))) as {
              error?: string;
            };
            const msg = body.error || `HTTP ${r.status}`;
            alert(
              substitute(
                t("web.jobs.restart_failed", "restart failed: {error}"),
                {
                  error: msg,
                },
              ),
            );
            return;
          }
          onRestarted();
        } catch (e) {
          alert(
            substitute(
              t("web.jobs.restart_failed", "restart failed: {error}"),
              {
                error: String(e),
              },
            ),
          );
        } finally {
          btn.disabled = false;
        }
      });
    });
}

/// Build the inline delete-button HTML for a terminal job row. The
/// row-level delete is for one-off cleanup. Bulk pruning lives in
/// the "Prune old…" dialog at the bottom of the table.
export function deleteButtonHtml(jobId: string): string {
  const label = t("web.jobs.delete", "delete");
  return `<button type="button" class="btn-row-delete" data-delete-job="${escapeHtml(jobId)}" title="${escapeHtml(label)}" aria-label="${escapeHtml(label)}">×</button>`;
}

/// Mirror of `attachCancelHandlers` for the row-level delete button.
/// Pops a confirmation before firing - delete is one-way and the row
/// goes away on success, so an accidental click on the wrong row
/// would be hard to recover from without a backup.
export function attachDeleteHandlers(
  root: HTMLElement,
  onDeleted: () => void,
): void {
  root
    .querySelectorAll<HTMLButtonElement>("[data-delete-job]")
    .forEach((btn) => {
      btn.addEventListener("click", async (e) => {
        e.stopPropagation();
        const id = btn.dataset.deleteJob;
        if (!id) return;
        const confirmMsg = substitute(
          t(
            "web.jobs.delete_confirm",
            "Delete job {id}? This cannot be undone.",
          ),
          { id },
        );
        if (!window.confirm(confirmMsg)) return;
        btn.disabled = true;
        try {
          const r = await fetch(`/jobs/${encodeURIComponent(id)}`, {
            method: "DELETE",
          });
          if (!r.ok) {
            const body = (await r.json().catch(() => ({}))) as {
              error?: string;
            };
            const msg = body.error || `HTTP ${r.status}`;
            alert(
              substitute(
                t("web.jobs.delete_failed", "delete failed: {error}"),
                { error: msg },
              ),
            );
            return;
          }
          onDeleted();
        } catch (e) {
          alert(
            substitute(t("web.jobs.delete_failed", "delete failed: {error}"), {
              error: String(e),
            }),
          );
        } finally {
          btn.disabled = false;
        }
      });
    });
}
