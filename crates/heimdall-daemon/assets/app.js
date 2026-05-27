(() => {
  const statusEl = document.getElementById("status");
  const jobsTbody = document.getElementById("jobs-tbody");
  const campaignsTbody = document.getElementById("campaigns-tbody");
  const dutsList = document.getElementById("duts-list");
  const tabs = document.querySelectorAll(".tab");
  const views = {
    jobs: document.getElementById("view-jobs"),
    campaigns: document.getElementById("view-campaigns"),
    duts: document.getElementById("view-duts"),
  };

  // ---------- i18n ----------
  // Catalog populated by initI18n() from /i18n.json. Until that resolves,
  // t() returns the literal key (or the fallback) so first paint isn't blank.
  let catalog = {};
  let activeLocale = "en";

  function detectLocale() {
    const params = new URLSearchParams(location.search);
    const fromQuery = params.get("lang");
    if (fromQuery) return fromQuery;
    try {
      const stored = localStorage.getItem("heimdall.lang");
      if (stored) return stored;
    } catch (_) { /* localStorage may be disabled */ }
    return navigator.language || "en";
  }

  function t(key, fallback) {
    return catalog[key] || fallback || key;
  }

  // Substitute {name} placeholders with values from `args`.
  function tr(key, args) {
    let s = t(key);
    if (args) {
      for (const name of Object.keys(args)) {
        s = s.split(`{${name}}`).join(String(args[name]));
      }
    }
    return s;
  }

  function applyStaticTranslations() {
    document.querySelectorAll("[data-i18n]").forEach((el) => {
      const key = el.getAttribute("data-i18n");
      const translated = catalog[key];
      if (translated) el.textContent = translated;
    });
    document.documentElement.setAttribute("lang", activeLocale);
  }

  async function initI18n() {
    const lang = detectLocale();
    try {
      const r = await fetch(`/i18n.json?lang=${encodeURIComponent(lang)}`);
      if (r.ok) {
        const body = await r.json();
        catalog = body;
        activeLocale = body._locale || "en";
        try { localStorage.setItem("heimdall.lang", activeLocale); } catch (_) {}
      }
    } catch (_) {
      // Network failure: leave catalog empty. Keys surface as fallback text.
    }
    applyStaticTranslations();
  }

  function shortId(s) { return (s || "").slice(0, 8); }
  function fmtTime(iso) { return iso ? iso.replace("T", " ").slice(0, 19) : "-"; }

  function jobStateLabel(state) {
    if (!state) return ["", "-"];
    const tag = state.state;
    if (tag === "done") {
      const v = state.detail && state.detail.kind;
      return [`state-done verdict-${v}`, `done/${v}`];
    }
    if (tag === "failed") {
      return ["state-failed", `failed: ${state.detail || ""}`];
    }
    return [`state-${tag}`, tag];
  }

  function renderJobs(jobs) {
    if (!jobs.length) {
      jobsTbody.innerHTML = `<tr class="empty"><td colspan="5">${escapeHtml(t("tui.empty.no_jobs", "no jobs"))}</td></tr>`;
      return;
    }
    jobsTbody.innerHTML = jobs.map(j => {
      const [cls, label] = jobStateLabel(j.state);
      const kindKind = (j.kind && j.kind.kind) || "?";
      return `
        <tr>
          <td class="id">${shortId(j.id)}</td>
          <td>${j.dut}</td>
          <td>${kindKind}</td>
          <td class="${cls}">${label}</td>
          <td>${fmtTime(j.created_at)}</td>
        </tr>`;
    }).join("");
  }

  function renderCampaigns(campaigns) {
    if (!campaigns.length) {
      campaignsTbody.innerHTML = `<tr class="empty"><td colspan="5">${escapeHtml(t("tui.empty.no_campaigns", "no campaigns"))}</td></tr>`;
      return;
    }
    campaignsTbody.innerHTML = campaigns.map(c => {
      const state = (c.state && c.state.state) || "?";
      const tpl = (c.template && c.template.kind) || "?";
      return `
        <tr>
          <td class="id">${shortId(c.id)}</td>
          <td>${c.dut}</td>
          <td>${tpl}</td>
          <td class="campaign-state-${state}">${state}</td>
          <td>${c.chip_serial || "-"}</td>
        </tr>`;
    }).join("");
  }

  async function refreshJobs() {
    try {
      const r = await fetch("/jobs");
      if (!r.ok) throw new Error(r.status);
      const body = await r.json();
      renderJobs(body.jobs || []);
    } catch (e) {
      console.warn("refreshJobs", e);
    }
  }

  async function refreshCampaigns() {
    try {
      const r = await fetch("/campaigns");
      if (!r.ok) throw new Error(r.status);
      const body = await r.json();
      renderCampaigns(body.campaigns || []);
    } catch (e) {
      console.warn("refreshCampaigns", e);
    }
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, c => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
    }[c]));
  }

  function renderDuts(duts) {
    if (!duts.length) {
      dutsList.innerHTML = `<p class="empty-block">${escapeHtml(t("web.duts.no_duts", "no DUTs configured"))}</p>`;
      return;
    }
    const jtagPrefix = t("web.duts.jtag_prefix", "jtag:");
    const showNetlist = t("web.duts.show_netlist", "Show netlist");
    // Status label is localized. CSS class stays in English (`connected`,
    // `disconnected`, `idle`) so the color rules don't need to change.
    const statusLabels = {
      connected: t("common.status.connected", "connected"),
      disconnected: t("common.status.disconnected", "disconnected"),
      idle: t("common.status.idle", "idle"),
    };
    dutsList.innerHTML = duts.map(d => {
      const id = escapeHtml(d.id);
      const kind = escapeHtml(d.kind);
      const serial = d.chip_serial ? escapeHtml(d.chip_serial) : "-";
      const jtag = (d.jtag && d.jtag.driver) ? escapeHtml(d.jtag.driver) : "-";
      const rawStatus = d.connection_status || "unknown";
      const statusClass = rawStatus === "unknown" ? "idle" : rawStatus;
      const statusLabel = statusLabels[statusClass];
      const hasNetlist = !!d.netlist;
      const netlistBlock = hasNetlist ? `
        <details class="netlist-panel">
          <summary>${escapeHtml(showNetlist)}</summary>
          <div class="netlist-svg">
            <img src="/duts/${encodeURIComponent(d.id)}/netlist.svg" alt="netlist for ${id}" />
          </div>
        </details>` : "";
      return `
        <div class="dut-card">
          <div class="dut-header">
            <span class="dut-id">${id}</span>
            <span class="dut-kind">${kind}</span>
            <span class="dut-serial">${serial}</span>
            <span class="dut-jtag">${escapeHtml(jtagPrefix)} ${jtag}</span>
            <span class="dut-status dut-status-${statusClass}">${escapeHtml(statusLabel)}</span>
          </div>
          ${netlistBlock}
        </div>`;
    }).join("");
  }

  async function refreshDuts() {
    try {
      const r = await fetch("/duts");
      if (!r.ok) throw new Error(r.status);
      const body = await r.json();
      renderDuts(body.duts || []);
    } catch (e) {
      console.warn("refreshDuts", e);
    }
  }

  function activate(view) {
    tabs.forEach(t => t.classList.toggle("active", t.dataset.view === view));
    Object.entries(views).forEach(([k, el]) => el.classList.toggle("hidden", k !== view));
  }

  tabs.forEach(t => t.addEventListener("click", () => activate(t.dataset.view)));
  document.addEventListener("keydown", (e) => {
    if (e.key === "1") activate("jobs");
    if (e.key === "2") activate("campaigns");
    if (e.key === "3") activate("duts");
  });

  function openSocket() {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const url = `${proto}://${location.host}/events`;
    let ws;
    try { ws = new WebSocket(url); } catch (e) {
      statusEl.textContent = t("common.status.disconnected", "disconnected");
      return;
    }
    ws.onopen = () => { statusEl.textContent = t("common.status.connected", "connected"); };
    ws.onclose = () => {
      statusEl.textContent = t("tui.disconnected_short", "disconnected, retrying...");
      setTimeout(openSocket, 2000);
    };
    ws.onerror = () => { statusEl.textContent = t("common.status.disconnected", "disconnected"); };
    ws.onmessage = (e) => {
      try {
        const ev = JSON.parse(e.data);
        const kind = ev.kind || "?";
        statusEl.textContent = tr("tui.event_label", { kind });
        if (kind.startsWith("job-") || kind.startsWith("lease-")) refreshJobs();
        if (kind.startsWith("campaign-")) refreshCampaigns();
        if (kind.startsWith("lease-")) refreshDuts();
      } catch (_) { /* ignore */ }
    };
  }

  // Wait for the catalog before painting, so first render isn't in English.
  initI18n().then(() => {
    refreshJobs();
    refreshCampaigns();
    refreshDuts();
    openSocket();
    setInterval(refreshJobs, 5000);
    setInterval(refreshCampaigns, 5000);
    // DUTs change only on daemon restart. Refresh less aggressively.
    setInterval(refreshDuts, 30000);
  });
})();
