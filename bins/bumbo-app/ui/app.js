"use strict";

let appState = null;
let presetModels = [];
let selectedRepoId = null;
let downloadsPollTimer = null;
let activityPollTimer = null;
let watchPollTimer = null;
let lastWatchFingerprint = null;

let currentView = "inspector"; // active workbench pane: inspector | editor | workspace | modules | models | plugins | mcp | codex
let forceHomeDashboard = false;
let isGenerating = false;
let selectedQuestionOpts = new Set();
let attachedFiles = []; // workspace rel paths
let attachedMedia = []; // MediaAttachment objects
let sideBySideDiff = false;
let lastRenderedChatKey = "";
let dragEnterDepth = 0;
let activeLightboxBlobUrl = null;
let artifactEditIds = new Set();
let pluginCatFilter = "all";
let depMgrFilter = null;
let installJobsPollTimer = null;
let installJobsDismissed = false;
const mcpProbeCache = new Map();

const expandedProjects = new Set();
let didInitProjectExpand = false;
const CUST_VIEWS = new Set(["codex", "modules", "workspace", "mcp"]);
const navHistory = [];
let navHistoryIdx = -1;
let isNavigatingHistory = false;

function normPath(p) {
  return (p || "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}

function fmtRelAge(iso) {
  if (!iso) return "";
  const ms = Date.now() - new Date(iso).getTime();
  if (!Number.isFinite(ms) || ms < 60000) return "now";
  const mins = Math.floor(ms / 60000);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h`;
  const days = Math.floor(hrs / 24);
  if (days < 14) return `${days}d`;
  return `${Math.floor(days / 7)}w`;
}

function fmtBytesShort(bytes) {
  const b = Number(bytes || 0);
  if (!b) return "";
  if (b >= 1024 ** 3) return `${(b / (1024 ** 3)).toFixed(1)} GB`;
  return `${Math.round(b / (1024 ** 2))} MB`;
}

// Multi-tab editor state
let openTabs = []; // { rel_path, kind, content, savedContent, raw_url, size_bytes, editing }
let activeTabPath = null;
let rafLineSync = null;
const codeSnippetsStore = new Map();

const pickerState = {
  a: { cat: "all", query: "", open: false },
  b: { cat: "all", query: "", open: false },
};

const HARNESS_MODULES = [
  { id: "fs_read", name: "Filesystem Read & Inspection", desc: "Read workspace files, list directories, search code, and extract offline PDF text." },
  { id: "fs_write", name: "File Mutations & Checkpoints", desc: "Write and edit workspace files with automatic pre-turn backups and diffs." },
  { id: "shell", name: "Shell & Auto-Verify", desc: "Run workspace shell commands, compilers, and test suites (cargo/pytest)." },
  { id: "rag", name: "Codex Offline RAG", desc: "Search the 100% offline BM25 + local embeddings workspace index." },
  { id: "subagents", name: "Research Subagents", desc: "Spawn isolated read-only subagents to summarize large files or PDFs." },
  { id: "web", name: "Web Search (Hybrid Mode)", desc: "Query Brave or DuckDuckGo search only when Hybrid Mode is active." },
];

const SVG = {
  folder: `<svg class="ico" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1.5 3.5h4.5l1.5 1.5h7v7.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-9Z"/></svg>`,
  file: `<svg class="ico" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M3.5 2h6l3 3v9h-9V2Z"/><path d="M9.5 2v3h3"/></svg>`,
  tool: `<svg class="ico" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M10.5 2.5a3.5 3.5 0 0 0-4.2 4.5L2.5 10.8 4.2 12.5 8 8.7a3.5 3.5 0 0 0 4.5-4.2l-1.8 1.8-1.4-1.4 1.2-2.4Z"/></svg>`,
  sendArrow: `<path d="M8 12.5V3.5M4.5 7 8 3.5 11.5 7"/>`,
  stopRect: `<rect x="4.5" y="4.5" width="7" height="7" rx="1" fill="currentColor" stroke="none"/>`,
};

async function api(path, opts = {}) {
  const res = await fetch(path, {
    headers: { "Content-Type": "application/json", ...(opts.headers || {}) },
    ...opts,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error(data.error || `HTTP ${res.status}`);
  }
  return data;
}

function showToast(msg, isError = false) {
  const box = document.getElementById("toast-box");
  box.textContent = msg;
  box.style.borderColor = isError ? "var(--red)" : "var(--accent)";
  box.classList.remove("hidden");
  clearTimeout(box._t);
  box._t = setTimeout(() => box.classList.add("hidden"), 4500);
}

function setValIfIdle(id, value) {
  const el = document.getElementById(id);
  if (el && document.activeElement !== el) {
    el.value = value ?? "";
  }
}

function esc(str) {
  return String(str ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fmtBytes(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = Number(bytes);
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i >= 2 ? 1 : 0)} ${units[i]}`;
}

function fmtNum(n) {
  return Number(n || 0).toLocaleString();
}

function timeGreeting() {
  const h = new Date().getHours();
  if (h < 12) return "Good morning,";
  if (h < 18) return "Good afternoon,";
  return "Good evening,";
}

function renderInlineMd(escapedText) {
  return escapedText
    .replace(/`([^`\n]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
}

function renderMarkdownLite(text, scopeId = "g") {
  const blocks = [];
  let src = String(text || "");

  src = src.replace(/```([\w-]*)\n([\s\S]*?)```/g, (_, lang, code) => {
    const cleanLang = (lang || "code").toLowerCase();
    const id = `cb_${scopeId}_${blocks.length}`;
    codeSnippetsStore.set(id, { lang: cleanLang, code });
    const previewBtn =
      cleanLang === "html" || cleanLang === "htm"
        ? `<button type="button" class="btn-ghost cb-preview-btn" data-cbid="${id}">Preview</button>`
        : "";
    const blockHtml = `<div class="code-block-wrap">
      <div class="code-block-head">
        <span>${esc(cleanLang)}</span>
        <span class="inline-row">
          ${previewBtn}
          <button type="button" class="btn-ghost cb-copy-btn" data-cbid="${id}">Copy</button>
          <button type="button" class="btn-ghost cb-apply-btn" data-cbid="${id}">Apply to Editor</button>
          <button type="button" class="btn-ghost cb-run-btn" data-cbid="${id}">Run</button>
        </span>
      </div>
      <pre><code class="lang-${esc(cleanLang)}">${esc(code)}</code></pre>
    </div>`;
    blocks.push(blockHtml);
    return `\n@@CODEBLOCK_${blocks.length - 1}@@\n`;
  });

  const lines = src.split("\n");
  const out = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (/^@@CODEBLOCK_\d+@@$/.test(trimmed)) {
      out.push(trimmed);
      i++;
      continue;
    }

    // GitHub Alerts: > [!NOTE] / [!TIP] / [!IMPORTANT] / [!WARNING] / [!CAUTION]
    const alertMatch = trimmed.match(/^>\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*(.*)$/i);
    if (alertMatch) {
      const kind = alertMatch[1].toLowerCase();
      const alertLines = [];
      if (alertMatch[2]) alertLines.push(renderInlineMd(esc(alertMatch[2])));
      i++;
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        alertLines.push(renderInlineMd(esc(lines[i].trim().replace(/^>\s?/, ""))));
        i++;
      }
      out.push(`<div class="md-alert md-alert-${kind}"><div class="md-alert-title">${esc(kind)}</div><div>${alertLines.join("<br/>")}</div></div>`);
      continue;
    }

    // Standard Blockquote
    if (trimmed.startsWith(">")) {
      const qLines = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        qLines.push(renderInlineMd(esc(lines[i].trim().replace(/^>\s?/, ""))));
        i++;
      }
      out.push(`<div class="md-quote">${qLines.join("<br/>")}</div>`);
      continue;
    }

    // GFM Table
    if (
      trimmed.startsWith("|") &&
      i + 1 < lines.length &&
      /^\|?\s*[-:| ]+\s*\|/.test(lines[i + 1].trim())
    ) {
      const splitCells = (row) =>
        row.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => renderInlineMd(esc(c.trim())));
      const headers = splitCells(trimmed);
      i += 2;
      const bodyRows = [];
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        bodyRows.push(splitCells(lines[i].trim()));
        i++;
      }
      const thead = `<thead><tr>${headers.map((h) => `<th>${h}</th>`).join("")}</tr></thead>`;
      const tbody = `<tbody>${bodyRows.map((r) => `<tr>${r.map((c) => `<td>${c}</td>`).join("")}</tr>`).join("")}</tbody>`;
      out.push(`<div class="md-table-wrap"><table class="md-table">${thead}${tbody}</table></div>`);
      continue;
    }

    // Task checklist
    const taskMatch = trimmed.match(/^[-*]\s+\[( |x|X)\]\s+(.*)$/);
    if (taskMatch) {
      const done = taskMatch[1].toLowerCase() === "x";
      out.push(`<div class="md-task-item"><span class="md-task-box">${done ? "[x]" : "[ ]"}</span><span>${renderInlineMd(esc(taskMatch[2]))}</span></div>`);
      i++;
      continue;
    }

    if (trimmed.startsWith("### ")) {
      out.push(`<h4>${renderInlineMd(esc(trimmed.slice(4)))}</h4>`);
    } else if (trimmed.startsWith("## ")) {
      out.push(`<h3>${renderInlineMd(esc(trimmed.slice(3)))}</h3>`);
    } else if (trimmed.startsWith("# ")) {
      out.push(`<h2>${renderInlineMd(esc(trimmed.slice(2)))}</h2>`);
    } else if (/^[-*]\s+/.test(trimmed)) {
      out.push(`<div>• ${renderInlineMd(esc(trimmed.replace(/^[-*]\s+/, "")))}</div>`);
    } else if (trimmed === "") {
      out.push(`<br/>`);
    } else {
      out.push(`<div>${renderInlineMd(esc(line))}</div>`);
    }
    i++;
  }

  let html = out.join("");
  blocks.forEach((b, idx) => {
    html = html.replace(`@@CODEBLOCK_${idx}@@`, b);
  });
  return html;
}

function renderDiffHtml(diffText) {
  if (!diffText) return "";
  if (sideBySideDiff) {
    const left = [];
    const right = [];
    for (const line of diffText.split("\n")) {
      if (line.startsWith("--- ") || line.startsWith("+++ ") || line.startsWith("@@")) {
        left.push(`<span class="diff-hunk">${esc(line)}</span>`);
        right.push(`<span class="diff-hunk">${esc(line)}</span>`);
      } else if (line.startsWith("-")) {
        left.push(`<span class="diff-del">${esc(line)}</span>`);
      } else if (line.startsWith("+")) {
        right.push(`<span class="diff-add">${esc(line)}</span>`);
      } else {
        left.push(`<span>${esc(line)}\n</span>`);
        right.push(`<span>${esc(line)}\n</span>`);
      }
    }
    return `<div class="diff-split"><pre class="diff-pre">${left.join("")}</pre><pre class="diff-pre">${right.join("")}</pre></div>`;
  }
  return diffText
    .split("\n")
    .map((line) => {
      if (line.startsWith("+") && !line.startsWith("+++")) return `<span class="diff-add">${esc(line)}</span>`;
      if (line.startsWith("-") && !line.startsWith("---")) return `<span class="diff-del">${esc(line)}</span>`;
      if (line.startsWith("@@")) return `<span class="diff-hunk">${esc(line)}</span>`;
      return `<span>${esc(line)}\n</span>`;
    })
    .join("");
}

function mapLegacyTheme(t) {
  return !t || t === "singularity_obsidian" ? "cinder_forge" : t;
}
function mapLegacyAccent(a) {
  return !a || a === "amber" ? "cinder" : a;
}

function syncPaneToggleStates() {
  const root = document.getElementById("app-root");
  if (!root) return;
  document.getElementById("toggle-sidebar-btn").classList.toggle("active", !root.classList.contains("sidebar-collapsed"));
  document.getElementById("toggle-workbench-btn").classList.toggle("active", !root.classList.contains("workbench-collapsed"));
  document.getElementById("toggle-explorer-btn").classList.toggle("active", !root.classList.contains("explorer-collapsed"));
}

function applyUiCustomization() {
  if (!appState?.settings?.ui) return;
  const ui = appState.settings.ui;
  const root = document.documentElement;
  root.setAttribute("data-theme", mapLegacyTheme(ui.theme));
  root.setAttribute("data-accent", mapLegacyAccent(ui.accent_color));
  root.setAttribute("data-scale", ui.font_scale || "default");
  syncPaneToggleStates();
}

function syncNavActiveState() {
  document.querySelectorAll(".nav-item[data-view]").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === currentView);
  });
  const isCust = CUST_VIEWS.has(currentView);
  document.getElementById("nav-customizations").classList.toggle("active", isCust);
  document.getElementById("cust-subnav").classList.toggle("hidden", !isCust);
  document.querySelectorAll(".cust-tab-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === currentView);
  });
  document.querySelectorAll(".wb-view-tab").forEach((b) => {
    const wv = b.dataset.wbview;
    const active = wv === "codex" ? isCust : wv === currentView;
    b.classList.toggle("active", active);
  });
}

function updateNavHistoryButtons() {
  document.getElementById("nav-back-btn").disabled = navHistoryIdx <= 0;
  document.getElementById("nav-fwd-btn").disabled = navHistoryIdx >= navHistory.length - 1;
}

async function navigateHistory(delta) {
  const targetIdx = navHistoryIdx + delta;
  if (targetIdx < 0 || targetIdx >= navHistory.length) return;
  navHistoryIdx = targetIdx;
  const entry = navHistory[targetIdx];
  isNavigatingHistory = true;
  try {
    forceHomeDashboard = !!entry.dash;
    if (entry.sid && appState?.settings?.active_session_id !== entry.sid) {
      await api("/api/sessions/select", { method: "POST", body: JSON.stringify({ session_id: entry.sid }) });
      await refreshState();
    }
    switchView(entry.view, false);
    renderHomeStage();
  } finally {
    isNavigatingHistory = false;
    updateNavHistoryButtons();
  }
}

function switchView(viewName, recordHistory = true) {
  if (viewName === "home") {
    renderHomeStage();
    if (window.innerWidth <= 1080) {
      document.getElementById("app-root").classList.add("sidebar-collapsed");
    }
    syncPaneToggleStates();
    return;
  }

  currentView = viewName;
  const appRoot = document.getElementById("app-root");
  appRoot.classList.remove("workbench-collapsed");

  document.querySelectorAll("#stage-main .view-pane").forEach((v) => {
    const isMatch = viewName === "inspector" ? v.id === "timeline-drawer" : v.id === `view-${viewName}`;
    v.classList.toggle("active", isMatch);
  });
  syncNavActiveState();
  syncPaneToggleStates();

  if (!isNavigatingHistory && recordHistory) {
    const sid = appState?.settings?.active_session_id || null;
    const prev = navHistory[navHistoryIdx];
    if (!prev || prev.view !== viewName || prev.sid !== sid || prev.dash !== forceHomeDashboard) {
      navHistory.splice(navHistoryIdx + 1);
      navHistory.push({ view: viewName, sid, dash: forceHomeDashboard });
      if (navHistory.length > 30) navHistory.shift();
      navHistoryIdx = navHistory.length - 1;
    }
  }
  updateNavHistoryButtons();
  if (window.innerWidth <= 1080) {
    appRoot.classList.add("sidebar-collapsed");
  }
  const titles = {
    inspector: "Artifacts & Diffs",
    workspace: "Projects & Rules",
    editor: "Code & Preview",
    modules: "Modules & Policy",
    models: "Forge & Models",
    plugins: "Plugins & Packages",
    mcp: "MCP Servers",
    codex: "Codex & Skills",
  };
  document.getElementById("hdr-view-title").textContent = titles[viewName] || "Cinder IDE";
  if (viewName === "models" && !presetModels.length) {
    loadComplexPresets().catch(() => {});
  }
}

function switchDrawerTab(tabName) {
  document.querySelectorAll(".tab-btn").forEach((b) => b.classList.toggle("active", b.dataset.tab === tabName));
  document.querySelectorAll(".tab-pane").forEach((p) => p.classList.toggle("active", p.id === `tab-${tabName}`));
}

function switchSettingsTab(tabName) {
  document.querySelectorAll(".settings-tab-btn").forEach((b) => b.classList.toggle("active", b.dataset.stab === tabName));
  document.querySelectorAll(".settings-pane").forEach((p) => p.classList.toggle("active", p.id === `stab-${tabName}`));
}

function openSettingsModal(tabName = null) {
  closeAllPopovers();
  if (tabName) switchSettingsTab(tabName);
  renderSettingsModal();
  document.getElementById("settings-modal").classList.remove("hidden");
}

function closeSettingsModal() {
  document.getElementById("settings-modal").classList.add("hidden");
}

function openLightboxImage(src, title) {
  if (activeLightboxBlobUrl) {
    URL.revokeObjectURL(activeLightboxBlobUrl);
    activeLightboxBlobUrl = null;
  }
  document.getElementById("lightbox-title").textContent = title || "Image Preview";
  document.getElementById("lightbox-body").innerHTML = `<img class="lightbox-img" src="${src}" alt="${esc(title)}" />`;
  document.getElementById("image-lightbox-modal").classList.remove("hidden");
}

function openLightboxHtml(htmlContent, title) {
  if (activeLightboxBlobUrl) {
    URL.revokeObjectURL(activeLightboxBlobUrl);
    activeLightboxBlobUrl = null;
  }
  activeLightboxBlobUrl = URL.createObjectURL(new Blob([htmlContent], { type: "text/html" }));
  document.getElementById("lightbox-title").textContent = title || "HTML Preview";
  document.getElementById("lightbox-body").innerHTML = `<iframe class="lightbox-frame" sandbox="allow-scripts" src="${activeLightboxBlobUrl}"></iframe>`;
  document.getElementById("image-lightbox-modal").classList.remove("hidden");
}

function closeLightbox() {
  document.getElementById("image-lightbox-modal").classList.add("hidden");
  document.getElementById("lightbox-body").innerHTML = "";
  if (activeLightboxBlobUrl) {
    URL.revokeObjectURL(activeLightboxBlobUrl);
    activeLightboxBlobUrl = null;
  }
}

function openCommandPalette() {
  document.getElementById("cmd-modal").classList.remove("hidden");
  const inp = document.getElementById("cmd-search-input");
  inp.value = "";
  renderCommandPalette("");
  inp.focus();
}

function closeCommandPalette() {
  document.getElementById("cmd-modal").classList.add("hidden");
}

function allReachableModels(respectFilter = true) {
  if (!appState) return [];
  const onlyReachable = respectFilter && (appState.settings.orchestrator?.only_show_reachable_models ?? true);
  const items = [];

  for (const m of appState.local_models) {
    if (m.is_embedding) continue;
    items.push({
      id: m.id,
      displayName: m.name,
      category: "local",
      providerLabel: "LOCAL GGUF",
      badgeClass: "badge-green",
      reachable: true,
      thinking:
        m.name.toLowerCase().includes("qwen3") ||
        m.name.toLowerCase().includes("gemma") ||
        m.name.toLowerCase().includes("r1"),
      tools: true,
      extraTag: m.quant,
      pctBar: Math.min(100, (m.vram_fit.estimated_total_mb / Math.max(1, appState.hardware.primary_vram_mb || 12288)) * 100),
      barClass: m.vram_fit.tier === "fits_in_vram" ? "meter-green" : m.vram_fit.tier === "partial_offload" ? "meter-amber" : "meter-red",
      rightMeta: `${fmtBytes(m.size_bytes)} · ${m.vram_fit.recommended_ngl === 99 ? "Full GPU" : `NGL ${m.vram_fit.recommended_ngl}`}`,
      searchBlob: `${m.id} ${m.name} local gguf ${m.quant} ${m.vram_fit.label}`.toLowerCase(),
    });
  }

  for (const c of appState.cloud_catalog) {
    if (onlyReachable && !c.reachable) continue;
    const q = appState.cloud_quotas.find((x) => x.model_id === c.id);
    const pct = q ? q.remaining_daily_pct : 100;
    const isGoogle = c.provider === "google_gemini";
    const providerLabel = isGoogle
      ? "GEMINI PRO"
      : c.provider === "anthropic"
      ? "ANTHROPIC"
      : c.provider === "open_router"
      ? "OPENROUTER"
      : "CLOUD";
    const badgeClass = !c.reachable
      ? "badge-muted"
      : isGoogle
      ? "badge-amber"
      : c.provider === "anthropic"
      ? "badge-purple"
      : "badge-blue";
    const ctxK = c.context_window >= 1000000 ? `${(c.context_window / 1000000).toFixed(0)}M ctx` : `${Math.round(c.context_window / 1000)}k ctx`;

    items.push({
      id: c.id,
      displayName: c.antigravity_alias || c.display_name,
      category: isGoogle ? "google" : "cloud",
      providerLabel,
      badgeClass,
      reachable: !!c.reachable,
      thinking: c.supports_thinking,
      tools: c.supports_tools,
      extraTag: ctxK,
      pctBar: pct,
      barClass: pct > 50 ? "meter-green" : pct > 20 ? "meter-amber" : "meter-red",
      rightMeta: c.reachable ? `${pct.toFixed(1)}% left` : c.reachability_reason || "Unavailable",
      searchBlob: `${c.id} ${c.display_name} ${c.antigravity_alias || ""} ${providerLabel} ${c.tier_note}`.toLowerCase(),
    });
  }
  return items;
}

async function refreshState(force = false) {
  appState = await api(force ? "/api/state?refresh=1" : "/api/state");
  renderAll();
}

function applyTurnPayload(res) {
  if (!appState) return refreshState();
  if (res.session) {
    appState.active_session = res.session;
    appState.settings.active_session_id = res.session.id;
    const idx = appState.sessions.findIndex((s) => s.id === res.session.id);
    const summary = {
      id: res.session.id,
      title: res.session.title,
      workspace_path: res.session.workspace_path,
      mode: res.session.mode,
      message_count: res.session.messages.length,
      updated_at: res.session.updated_at,
    };
    if (idx >= 0) appState.sessions[idx] = summary;
    else appState.sessions.unshift(summary);
  }
  if (res.cloud_quotas) appState.cloud_quotas = res.cloud_quotas;
  if (res.server_status) appState.server_status = res.server_status;
  if (res.privacy_audit) appState.privacy_audit = res.privacy_audit;
  if (res.workspace_tree) appState.workspace_tree = res.workspace_tree;
  renderAll();
}

function renderAll() {
  if (!appState) return;
  applyUiCustomization();
  syncNavActiveState();
  renderTopAndSidebar();
  renderHomeStage();
  renderWorkspaceView();
  renderFileTree();
  renderEditorTabsAndPane();
  renderModulesView();
  renderModelsRepoView();
  renderPluginsView();
  renderMcpView();
  renderCodexView();
  renderTimelineDrawer();
  renderSettingsModal();
}

function renderConvRowHtml(s, activeSid) {
  const active = s.id === activeSid;
  const age = fmtRelAge(s.updated_at);
  return `<div class="conv-row session-row ${active ? "active" : ""}" data-sid="${esc(s.id)}">
    <span class="conv-title">${esc(s.title)}</span>
    <span class="conv-right">
      ${age ? `<span class="conv-age">${esc(age)}</span>` : ""}
      <button type="button" class="btn-ghost del-sess-btn" data-sid="${esc(s.id)}" title="Delete conversation">×</button>
    </span>
  </div>`;
}

function wireSessionRowClicks(container) {
  container.querySelectorAll(".session-row").forEach((el) => {
    el.addEventListener("click", (e) => {
      if (e.target.closest(".del-sess-btn")) return;
      selectSession(el.dataset.sid);
    });
  });
  container.querySelectorAll(".del-sess-btn").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      deleteSession(btn.dataset.sid);
    });
  });
}

function renderProjectChatsTree(filterQ) {
  const activeNorm = normPath(appState.settings.active_workspace_path);
  if (!didInitProjectExpand && activeNorm) {
    didInitProjectExpand = true;
    expandedProjects.add(activeNorm);
  }

  const activeSid = appState.settings.active_session_id;
  const allSessions = (appState.sessions || []).slice().sort((a, b) => {
    return new Date(b.updated_at || 0).getTime() - new Date(a.updated_at || 0).getTime();
  });
  const matching = allSessions.filter((s) => !filterQ || s.title.toLowerCase().includes(filterQ));

  const byWs = new Map();
  for (const w of appState.workspaces || []) {
    byWs.set(normPath(w.path), []);
  }
  const uncategorized = [];
  for (const s of matching) {
    const np = normPath(s.workspace_path);
    if (byWs.has(np)) {
      byWs.get(np).push(s);
    } else {
      uncategorized.push(s);
    }
  }

  const treeEl = document.getElementById("projects-tree-list");
  if (!(appState.workspaces || []).length) {
    treeEl.innerHTML = `<div class="sessions-empty">No projects yet.</div>`;
  } else {
    treeEl.innerHTML = (appState.workspaces || [])
      .map((w) => {
        const key = normPath(w.path);
        const list = byWs.get(key) || [];
        const isActive = key === activeNorm;
        const isOpen = filterQ ? list.length > 0 : expandedProjects.has(key);
        const convHtml = isOpen
          ? list.length
            ? `<div class="proj-convs">${list.map((s) => renderConvRowHtml(s, activeSid)).join("")}</div>`
            : `<div class="proj-convs"><div class="sessions-empty" style="padding:4px 8px">No chats in project yet</div></div>`
          : "";
        return `<div class="proj-group">
          <button type="button" class="proj-row ${isActive ? "active" : ""}" data-projpath="${esc(w.path)}" data-projkey="${esc(key)}" title="${esc(w.path)}">
            <span class="proj-chev">${isOpen ? "▾" : "▸"}</span>
            ${SVG.folder}
            <span class="proj-name">${esc(w.name)}</span>
            ${list.length ? `<span class="proj-count">${list.length}</span>` : ""}
            ${!isActive && (appState.workspaces || []).length > 1 ? `<span role="button" class="btn-ghost del-proj-btn" data-delws="${esc(w.path)}" title="Remove project from sidebar">×</span>` : ""}
          </button>
          ${convHtml}
        </div>`;
      })
      .join("");

    treeEl.querySelectorAll(".del-proj-btn").forEach((delBtn) => {
      delBtn.addEventListener("click", async (e) => {
        e.stopPropagation();
        await api("/api/workspace/remove", { method: "POST", body: JSON.stringify({ path: delBtn.dataset.delws }) });
        await refreshState();
        showToast("Removed project from sidebar.");
      });
    });

    treeEl.querySelectorAll(".proj-row").forEach((btn) => {
      btn.addEventListener("click", async (e) => {
        if (e.target.closest(".del-proj-btn")) return;
        const key = btn.dataset.projkey;
        const wpath = btn.dataset.projpath;
        if (key !== activeNorm) {
          expandedProjects.add(key);
          await selectWorkspace(wpath);
        } else {
          if (expandedProjects.has(key)) expandedProjects.delete(key);
          else expandedProjects.add(key);
          renderProjectChatsTree(filterQ);
        }
      });
    });
    wireSessionRowClicks(treeEl);
  }

  // Recent Conversations (uncategorized + chats inside currently collapsed projects)
  const sessList = document.getElementById("sessions-list");
  const hiddenInCollapsed = matching.filter((s) => {
    const np = normPath(s.workspace_path);
    if (!byWs.has(np)) return true;
    const wsOpen = filterQ ? (byWs.get(np) || []).length > 0 : expandedProjects.has(np);
    return !wsOpen;
  });
  const shown = hiddenInCollapsed.slice(0, 10);
  if (!shown.length) {
    sessList.innerHTML = `<div class="sessions-empty">${matching.length ? "All chats shown in Projects above." : "Your conversations will appear here."}</div>`;
  } else {
    sessList.innerHTML = shown.map((s) => renderConvRowHtml(s, activeSid)).join("");
    wireSessionRowClicks(sessList);
  }
}

function renderTopAndSidebar() {
  const activeWs = appState.workspaces.find(
    (w) => normPath(w.path) === normPath(appState.settings.active_workspace_path)
  );
  document.getElementById("hdr-ws-pill").textContent = activeWs ? activeWs.name : appState.settings.active_workspace_path;

  const audit = appState.privacy_audit;
  const gpu = appState.hardware.gpus[0];
  const gpuLabel = gpu ? gpu.name.replace("NVIDIA GeForce ", "") : "CPU Mode";
  document.getElementById("footer-privacy-dot").classList.toggle("hybrid", audit.privacy_mode !== "strict_local");
  document.getElementById("footer-privacy-title").textContent = audit.privacy_mode === "strict_local" ? "Local" : "Hybrid";
  document.getElementById("privacy-footer-btn").title =
    (audit.privacy_mode === "strict_local"
      ? "Strict Local (0 external traffic)"
      : `Hybrid Mode (${audit.external_requests_allowed} external calls)`) +
    (gpu ? ` · ${gpuLabel} ${gpu.vram_used_mb}/${gpu.vram_total_mb} MB` : "");
  document.getElementById("footer-hw-sub").textContent = gpu
    ? `${gpuLabel} · ${gpu.vram_used_mb}/${gpu.vram_total_mb} MB`
    : `${appState.hardware.system_ram_mb} MB RAM`;

  const dot = document.getElementById("top-privacy-dot");
  dot.classList.toggle("hybrid", audit.privacy_mode !== "strict_local");

  const reachableCloud = appState.cloud_quotas.filter((q) => q.reachable);
  const activeCloud = reachableCloud.find((q) => q.model_id === appState.settings.active_model_id) || reachableCloud[0];
  document.getElementById("top-quota-text").textContent =
    audit.privacy_mode === "strict_local"
      ? `${appState.local_models.filter((m) => !m.is_embedding).length} GGUFs`
      : activeCloud
      ? `${activeCloud.remaining_daily_pct.toFixed(0)}% Quota`
      : "Local";

  // Local Model VRAM Guard
  const srv = appState.server_status || {};
  const guardBtn = document.getElementById("engine-guard-btn");
  const guardLabel = document.getElementById("guard-label");
  const guardDetail = document.getElementById("guard-status-detail");
  const guardStopBtn = document.getElementById("guard-stop-all-btn");
  if (srv.running) {
    guardBtn.classList.add("is-running");
    guardBtn.classList.remove("is-stopped");
    const shortModel = (srv.model_name || "Local").split(",")[0].slice(0, 18);
    const vramTxt = srv.vram_bytes > 0 ? ` · ${fmtBytesShort(srv.vram_bytes)}` : "";
    guardLabel.textContent = `Forging: ${shortModel}${vramTxt} ■`;
    const modelsList = (srv.loaded_models && srv.loaded_models.length ? srv.loaded_models : [srv.model_name || "Local"]).join(", ");
    guardDetail.innerHTML = `<span class="badge badge-amber">Loaded in VRAM</span> ${esc(srv.backend)} · <code>${esc(modelsList)}</code>${srv.vram_bytes > 0 ? ` (${fmtBytesShort(srv.vram_bytes)})` : ""}`;
    guardStopBtn.disabled = false;
  } else {
    guardBtn.classList.remove("is-running");
    guardBtn.classList.add("is-stopped");
    guardLabel.textContent = "Forge Cold";
    guardDetail.textContent = "Forge is cold (llama-server off & Ollama /api/ps empty). 0 MB held in VRAM.";
    guardStopBtn.disabled = true;
  }
  document.getElementById("chk-unload-after-turn").checked = !!appState.settings.ui?.unload_after_turn;
  const idleM = Number(appState.settings.idle_unload_minutes || 0);
  document.getElementById("guard-idle-note").textContent = idleM > 0 ? `Idle quench: ${idleM}m` : "Idle quench: Off";
  syncPaneToggleStates();

  const artCount = (appState.active_session?.artifacts || []).length;
  const artBadge = document.getElementById("artifacts-count-badge");
  artBadge.textContent = String(artCount);
  artBadge.classList.toggle("hidden", artCount === 0);

  renderBumboDropdown(activeWs, audit, gpu, gpuLabel);

  // Downloads & Runtime Popover
  const downloads = appState.downloads || [];
  const activeDlCount = downloads.filter((d) => d.status === "downloading" || d.status === "queued" || d.status === "verifying").length;
  const badge = document.getElementById("downloads-count-badge");
  badge.textContent = String(activeDlCount);
  badge.classList.toggle("hidden", activeDlCount === 0);

  const dlList = document.getElementById("downloads-popover-list");
  if (!downloads.length) {
    dlList.innerHTML = `<div class="muted-text" style="padding:8px 0">No active model downloads.</div>`;
  } else {
    dlList.innerHTML = downloads
      .slice(0, 6)
      .map((j) => {
        const pct = j.total_bytes > 0 ? Math.min(100, (j.bytes_downloaded / j.total_bytes) * 100) : 0;
        const canCancel = j.status === "downloading" || j.status === "queued";
        return `<div class="dl-row">
          <div class="card-title-row">
            <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(j.filename)}</span>
            <span class="badge badge-muted">${esc(j.status)}</span>
          </div>
          <div class="muted-text">${fmtBytes(j.bytes_downloaded)} of ${fmtBytes(j.total_bytes)} · ${pct.toFixed(0)}%</div>
          <div class="meter-track"><div class="meter-fill meter-blue" style="width:${pct.toFixed(1)}%"></div></div>
          ${canCancel ? `<div class="inline-row mt-4"><button type="button" class="btn-small cancel-dl-btn" data-jid="${esc(j.id)}">Pause / Cancel</button></div>` : ""}
        </div>`;
      })
      .join("");
    dlList.querySelectorAll(".cancel-dl-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        await api("/api/models/hf/cancel", { method: "POST", body: JSON.stringify({ job_id: b.dataset.jid }) });
        refreshState();
      });
    });
  }

  const eng = appState.engine_status;
  const rtPill = document.getElementById("runtime-status-pill");
  const rtDesc = document.getElementById("runtime-desc-text");
  const rtBtn = document.getElementById("quick-setup-runtime-btn");
  if (eng.llama_server_installed) {
    rtPill.className = "badge badge-green";
    rtPill.textContent = "Installed";
    rtDesc.textContent = `llama-server ready${eng.ollama_loopback_detected ? ` + Ollama (${eng.ollama_models.length} models)` : ""}.`;
    rtBtn.textContent = "Reinstall CUDA 12";
  } else if (eng.ollama_loopback_detected) {
    rtPill.className = "badge badge-blue";
    rtPill.textContent = "Ollama Active";
    rtDesc.textContent = `Using local Ollama (${eng.ollama_models.length} models). Install llama.cpp for full custom GGUF + mmproj support.`;
    rtBtn.textContent = "Install llama.cpp";
  } else {
    rtPill.className = "badge badge-amber";
    rtPill.textContent = "Not installed";
    rtDesc.textContent = "llama.cpp is the engine that runs your downloaded GGUFs locally with partial GPU/RAM offload.";
    rtBtn.textContent = "Setup runtime (CUDA 12)";
  }

  const filterQ = (document.getElementById("session-filter-input").value || "").trim().toLowerCase();
  renderProjectChatsTree(filterQ);
}

function renderChatChangesStrip(sess) {
  const strip = document.getElementById("chat-changes-strip");
  const sumEl = document.getElementById("chat-changes-summary");
  const taskEl = document.getElementById("chat-task-status");
  if (!strip || !sumEl || !taskEl) return;

  const cps = sess?.checkpoints || [];
  let fileSet = new Set();
  let adds = 0;
  let dels = 0;
  for (const cp of cps) {
    for (const f of cp.files || []) {
      fileSet.add(f.rel_path);
      for (const line of (f.diff || "").split("\n")) {
        if (line.startsWith("+") && !line.startsWith("+++")) adds++;
        else if (line.startsWith("-") && !line.startsWith("---")) dels++;
      }
    }
  }

  const show = fileSet.size > 0 || isGenerating;
  strip.classList.toggle("hidden", !show);
  sumEl.innerHTML = `${fileSet.size} file${fileSet.size === 1 ? "" : "s"} changed ${
    adds || dels ? `<span style="color:var(--green)">+${adds}</span> <span style="color:var(--red)">-${dels}</span>` : ""
  }`;
  taskEl.classList.toggle("hidden", !isGenerating);
  if (isGenerating) {
    taskEl.textContent = "Forging...";
  }
}

function renderHomeStage() {
  const sess = appState.active_session;
  const hasMsgs = !!(sess && sess.messages.length > 0);
  const showHomeMode = (!hasMsgs || forceHomeDashboard) && !isGenerating;

  const viewHome = document.getElementById("view-home");
  const hero = document.getElementById("home-hero");
  const chatStream = document.getElementById("chat-stream");
  const chatUtilBar = document.getElementById("chat-utility-bar");
  const anchor = document.getElementById("composer-anchor");
  const quickNav = document.getElementById("home-quick-nav");
  const dash = document.getElementById("home-dashboard");
  const resumePill = document.getElementById("home-resume-chat-pill");

  document.getElementById("hero-greeting").textContent = timeGreeting();

  viewHome.classList.toggle("is-home-empty", showHomeMode);
  hero.classList.toggle("hidden", !showHomeMode);
  quickNav.classList.toggle("hidden", !showHomeMode);
  dash.classList.toggle("hidden", !showHomeMode);
  resumePill.classList.toggle("hidden", !(forceHomeDashboard && hasMsgs));
  chatUtilBar.classList.toggle("hidden", showHomeMode);
  chatStream.classList.toggle("hidden", showHomeMode);
  anchor.classList.toggle("composer-centered", showHomeMode);
  anchor.classList.toggle("composer-docked", !showHomeMode);
  document.getElementById("chat-util-title").textContent = sess?.title || "Active Conversation";

  // Request Review + Thinking + Modules composer badges
  const orch = appState.settings.orchestrator || {};
  const askReview = !!orch.review_before_execute || !appState.settings.auto_approve_edits;
  document.getElementById("review-gate-label").textContent = askReview ? "Always Ask" : "Auto";
  document.getElementById("review-gate-pill-btn").classList.toggle("active", askReview);

  const effort = appState.settings.ui?.thinking_effort || "standard";
  const effortLabel = effort === "off" ? "Off" : effort === "deep" ? "Deep" : "Standard";
  document.getElementById("thinking-effort-label").textContent = effortLabel;
  document.getElementById("thinking-pill-btn").classList.toggle("active", effort !== "off");

  const enabledMods = appState.settings.ui?.enabled_modules || [];
  document.getElementById("modules-count-badge").textContent = String(enabledMods.length);

  renderComposerModeAndPickers();
  renderAttachedChips();
  renderChatChangesStrip(sess);

  // Collapsible Home Readiness Summary + Drawer Cards
  if (showHomeMode) {
    const gpu = appState.hardware.gpus[0];
    const srv = appState.server_status;
    const reachableCount = appState.cloud_catalog.filter((c) => c.reachable).length;
    const localCount = appState.local_models.filter((m) => !m.is_embedding).length;
    const activeWs = appState.workspaces.find((w) => normPath(w.path) === normPath(appState.settings.active_workspace_path));
    const gpuShort = gpu ? gpu.name.replace("NVIDIA GeForce ", "") : "CPU";
    document.getElementById("home-status-summary-text").textContent =
      `${activeWs ? activeWs.name : "Workspace"} · ${gpuShort}${gpu ? ` (${gpu.vram_used_mb}/${gpu.vram_total_mb} MB)` : ""} · ${srv.running ? `Forging ${srv.model_name || "Local"}` : "Forge Cold"} · ${localCount} GGUFs · ${reachableCount} Cloud`;

    const projGrid = document.getElementById("home-projects-grid");
    projGrid.innerHTML = appState.workspaces
      .map((w) => {
        const active = normPath(w.path) === normPath(appState.settings.active_workspace_path);
        return `<button type="button" class="project-card ${active ? "active" : ""}" data-wspath="${esc(w.path)}">
          <div class="card-title-row" style="margin-bottom:0">
            <span>${esc(w.name)}</span>
            ${active ? '<span class="badge badge-amber">Active</span>' : w.is_uni_project ? '<span class="badge badge-muted">Uni</span>' : ""}
          </div>
          <div class="muted-text" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(w.path)}</div>
        </button>`;
      })
      .join("");
    projGrid.querySelectorAll(".project-card").forEach((b) => {
      b.addEventListener("click", () => selectWorkspace(b.dataset.wspath));
    });

    document.getElementById("home-readiness-grid").innerHTML = `
      <div class="readiness-card">
        <div class="card-title-row"><span>GPU &amp; VRAM</span><span class="badge badge-green">Ready</span></div>
        <div class="muted-text">${gpu ? esc(gpu.name.replace("NVIDIA GeForce ", "")) : "CPU"} · ${gpu ? `${gpu.vram_used_mb} / ${gpu.vram_total_mb} MB` : "No discrete GPU"}</div>
      </div>
      <div class="readiness-card">
        <div class="card-title-row"><span>Cinder Forge</span><span class="badge ${srv.running ? "badge-amber" : "badge-green"}">${srv.running ? "Forging" : "Cold"}</span></div>
        <div class="muted-text">${srv.running ? esc(srv.model_name || "Active") : `${localCount} local GGUFs ready on disk`}</div>
      </div>
      <div class="readiness-card">
        <div class="card-title-row"><span>Cloud &amp; Quota</span><span class="badge badge-amber">${reachableCount} Reachable</span></div>
        <div class="muted-text">${appState.privacy_audit.privacy_mode === "strict_local" ? "Strict Local (0 external)" : "Hybrid Mode · Live verified"}</div>
      </div>
    `;
  }

  renderChatMessagesAndBanners();
}

function renderComposerModeAndPickers(preserveSelectValue = false) {
  const modeSel = document.getElementById("mode-select");
  const mode = preserveSelectValue
    ? modeSel.value || appState.active_session?.mode || "auto"
    : appState.active_session?.mode || modeSel.value || "auto";
  if (document.activeElement !== modeSel) modeSel.value = mode;

  const isOrch = mode === "orchestrator";
  const isCompare = mode === "compare";
  const roleA = document.getElementById("model-picker-role");
  const roleB = document.getElementById("model-b-picker-role");
  const wrapB = document.getElementById("model-b-wrap");
  const connB = document.getElementById("model-b-connector");

  roleA.textContent = "Planner";
  roleA.classList.toggle("hidden", !isOrch);
  wrapB.classList.toggle("hidden", !(isOrch || isCompare));
  connB.textContent = isOrch ? "→" : "vs";
  roleB.textContent = isOrch ? "Executor" : "Model B";

  renderModelPicker("a");
  renderModelPicker("b");
}

function renderModelPicker(which) {
  const mode = document.getElementById("mode-select").value;
  const isOrch = mode === "orchestrator";
  const models = allReachableModels(true);
  const activeId =
    which === "a"
      ? isOrch
        ? appState.settings.orchestrator.planner_model_id
        : appState.settings.active_model_id
      : isOrch
      ? appState.settings.orchestrator.executor_model_id
      : appState.settings.compare_model_b_id;

  const activeItem = models.find((m) => m.id === activeId) || models[0];
  const badgeEl = document.getElementById(which === "a" ? "model-picker-badge" : "model-b-picker-badge");
  const labelEl = document.getElementById(which === "a" ? "model-picker-label" : "model-b-picker-label");
  if (activeItem) {
    labelEl.textContent = activeItem.displayName;
    badgeEl.textContent = activeItem.category === "local" ? "Local" : activeItem.extraTag;
  } else {
    labelEl.textContent = "No reachable model";
    badgeEl.textContent = "None";
  }

  const st = pickerState[which];
  const filtered = models.filter((m) => {
    if (st.cat !== "all" && m.category !== st.cat) return false;
    if (st.query.trim()) {
      const terms = st.query.trim().toLowerCase().split(/\s+/);
      return terms.every((t) => m.searchBlob.includes(t));
    }
    return true;
  });

  const listEl = document.getElementById(which === "a" ? "model-picker-list" : "model-b-picker-list");
  if (!filtered.length) {
    listEl.innerHTML = `<div class="muted-text" style="padding:12px;text-align:center">No reachable models match. Configure API keys or GGUFs in Model Repository.</div>`;
    return;
  }

  listEl.innerHTML = filtered
    .map((m) => {
      const sel = m.id === (activeItem?.id || activeId) ? "selected" : "";
      return `<button type="button" class="picker-item ${sel}" data-mid="${esc(m.id)}">
        <div class="picker-item-top">
          <span>${esc(m.displayName)}</span>
          <span class="badge ${m.badgeClass}">${esc(m.providerLabel)}</span>
        </div>
        <div class="picker-item-meta">
          <span>${esc(m.extraTag)}</span>
          ${m.thinking ? '<span class="badge badge-purple">Thinking</span>' : ""}
          ${m.tools ? '<span class="badge badge-muted">Tools</span>' : ""}
          <span style="margin-left:auto">${esc(m.rightMeta)}</span>
        </div>
        <div class="picker-item-bar"><div class="meter-fill ${m.barClass}" style="width:${m.pctBar.toFixed(1)}%"></div></div>
      </button>`;
    })
    .join("");

  listEl.querySelectorAll(".picker-item").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const chosenId = btn.dataset.mid;
      closeAllPopovers();
      const curMode = document.getElementById("mode-select").value;
      if (curMode === "orchestrator") {
        const orch = { ...appState.settings.orchestrator };
        if (which === "a") orch.planner_model_id = chosenId;
        else orch.executor_model_id = chosenId;
        await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ orchestrator: orch }) });
      } else {
        const field = which === "a" ? "active_model_id" : "compare_model_b_id";
        await api("/api/settings/harness", { method: "POST", body: JSON.stringify({ [field]: chosenId }) });
      }
      await refreshState();
    });
  });
}

function buildChatSignature(sess, ui) {
  if (!sess) return "none";
  const last = sess.messages[sess.messages.length - 1];
  const cpStatus = (sess.checkpoints || []).map((c) => c.files.map((f) => f.status).join(",")).join("|");
  return [
    sess.id,
    sess.messages.length,
    last?.id || "",
    last?.content?.length || 0,
    last?.plan_artifact?.status || "",
    cpStatus,
    ui.wrap_code_blocks !== false,
    ui.show_token_metrics !== false,
    sideBySideDiff,
  ].join("::");
}

function renderChatMessagesAndBanners() {
  const stream = document.getElementById("chat-stream");
  const sess = appState.active_session;
  const ui = appState.settings.ui || {};
  const wrapClass = ui.wrap_code_blocks === false ? "nowrap-code" : "";
  const showMetrics = ui.show_token_metrics !== false;

  const chatKey = buildChatSignature(sess, ui);
  if (chatKey !== lastRenderedChatKey) {
    lastRenderedChatKey = chatKey;
    const wasNearBottom = stream.scrollHeight - stream.scrollTop - stream.clientHeight < 120;
    const prevCount = Number(stream.dataset.msgCount || "0");

    // Preserve chat snippets store cleanly once for all messages
    for (const k of Array.from(codeSnippetsStore.keys())) {
      if (k.startsWith("cb_msg")) codeSnippetsStore.delete(k);
    }

    if (!sess || !sess.messages.length) {
      stream.innerHTML = "";
      stream.dataset.msgCount = "0";
    } else {
      stream.innerHTML = sess.messages
        .map((m, idx) => {
          if (m.role === "user") {
            const attFilesHtml = (m.attached_files || []).length
              ? `<div class="attached-chips mb-6">${m.attached_files.map((f) => `<span class="file-chip">@${esc(f)}</span>`).join("")}</div>`
              : "";
            const imgAtts = (m.attachments || []).filter((a) => a.mime_type?.startsWith("image/") && a.data_base64);
            const otherAtts = (m.attachments || []).filter((a) => !(a.mime_type?.startsWith("image/") && a.data_base64));
            const galleryHtml = imgAtts.length
              ? `<div class="msg-att-gallery">${imgAtts
                  .map((a) => {
                    const src = `data:${esc(a.mime_type)};base64,${a.data_base64}`;
                    return `<img class="msg-att-img" src="${src}" alt="${esc(a.name)}" data-title="${esc(a.name)}" />`;
                  })
                  .join("")}</div>`
              : "";
            const otherChips = otherAtts.length
              ? `<div class="attached-chips mb-6">${otherAtts.map((a) => `<span class="file-chip">${esc(a.name)} (${fmtBytes(a.size_bytes)})</span>`).join("")}</div>`
              : "";
            return `<div class="msg-card msg-user">
              <div class="msg-meta">
                <span>You</span>
                <button type="button" class="btn-ghost rewind-btn" data-midx="${idx}" title="Edit prompt &amp; rewind conversation/checkpoints to here">Edit &amp; Rewind</button>
              </div>
              ${galleryHtml}${attFilesHtml}${otherChips}
              <div class="msg-body ${wrapClass}">${renderMarkdownLite(m.content, `msg${idx}`)}</div>
            </div>`;
          }

          const orchHtml = (m.orchestrator_phases || []).length
            ? `<div class="orch-timeline">${m.orchestrator_phases
                .map(
                  (p, pi) =>
                    `${pi > 0 ? '<span class="muted-text">→</span>' : ""}<span class="orch-phase-chip"><span class="badge badge-amber">${esc(
                      p.phase.toUpperCase()
                    )}</span><span>${esc(p.model_id)}</span>${p.latency_ms ? `<span class="muted-text">(${p.latency_ms}ms)</span>` : ""}</span>`
                )
                .join("")}</div>`
            : "";

          const thinkingHtml = m.thinking
            ? `<details class="thinking-box"><summary>Thought Process</summary><div class="thinking-content">${esc(m.thinking)}</div></details>`
            : "";

          const toolsHtml = (m.tool_calls || [])
            .map((tc) => {
              const diffBlock = tc.diff ? `<pre class="diff-pre">${renderDiffHtml(tc.diff)}</pre>` : "";
              return `<details class="tool-step">
                <summary>
                  <span class="inline-row">${SVG.tool} <span>${esc(tc.name)}</span> ${tc.file_path ? `<span class="muted-text">${esc(tc.file_path)}</span>` : ""}</span>
                  <span class="badge ${tc.status === "error" || tc.status === "rejected" ? "badge-red" : "badge-green"}">${esc(tc.status)}</span>
                </summary>
                ${diffBlock}
                <div class="tool-step-body">${esc(tc.output)}</div>
              </details>`;
            })
            .join("");

          const bodyHtml = m.compare_partner
            ? `<div class="compare-grid">
                <div class="compare-col">
                  <div class="msg-meta"><strong>${esc(m.model_id)}</strong><span>${m.latency_ms || 0}ms</span></div>
                  <div class="msg-body ${wrapClass}">${renderMarkdownLite(m.content, `msg${idx}a`)}</div>
                </div>
                <div class="compare-col">
                  <div class="msg-meta"><strong>${esc(m.compare_partner.model_id)}</strong><span>${m.compare_partner.latency_ms}ms · ${m.compare_partner.total_tokens} tok</span></div>
                  <div class="msg-body ${wrapClass}">${renderMarkdownLite(m.compare_partner.content, `msg${idx}b`)}</div>
                </div>
              </div>`
            : `<div class="msg-body ${wrapClass}">${renderMarkdownLite(m.content, `msg${idx}`)}</div>`;

          const planHtml = m.plan_artifact
            ? renderPlanCardHtml(sess.active_plan && sess.active_plan.id === m.plan_artifact.id ? sess.active_plan : m.plan_artifact)
            : "";

          const cp = m.checkpoint_id ? (sess.checkpoints || []).find((c) => c.id === m.checkpoint_id) : null;
          const cpHtml = cp && cp.files.length ? renderCheckpointInlineHtml(cp) : "";

          const artChip = m.artifact_id
            ? `<div class="mt-4"><button type="button" class="btn-small open-art-drawer-btn">View Updated Artifact →</button></div>`
            : "";

          const metaRight = showMetrics
            ? `<span>${m.latency_ms || 0}ms · ${(m.prompt_tokens || 0) + (m.completion_tokens || 0)} tok</span>`
            : "";

          return `<div class="msg-card">
            <div class="msg-meta"><span><strong>${esc(m.model_id || "assistant")}</strong></span>${metaRight}</div>
            ${orchHtml}
            ${thinkingHtml}
            ${toolsHtml}
            ${bodyHtml}
            ${artChip}
            ${planHtml}
            ${cpHtml}
          </div>`;
        })
        .join("");

      stream.dataset.msgCount = String(sess.messages.length);
      if (wasNearBottom || sess.messages.length > prevCount) {
        stream.scrollTop = stream.scrollHeight;
      }
    }

    stream.querySelectorAll(".rewind-btn").forEach((b) => {
      b.addEventListener("click", () => rewindToMessage(Number(b.dataset.midx)));
    });
    stream.querySelectorAll(".msg-att-img").forEach((img) => {
      img.addEventListener("click", () => openLightboxImage(img.src, img.dataset.title || "Attached Image"));
    });
    stream.querySelectorAll(".open-art-drawer-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        const uiNext = { ...appState.settings.ui, show_right_workbench: true };
        await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ ui: uiNext }) });
        await refreshState();
        switchDrawerTab("artifacts");
      });
    });

    wireCodeBlockButtons(stream);
    wireCheckpointButtons(stream);

    stream.querySelectorAll(".plan-card").forEach((card) => {
      const execBtn = card.querySelector(".plan-exec-btn");
      const revBtn = card.querySelector(".plan-revise-btn");
      const mdEl = card.querySelector(".plan-md-editor");
      const fbEl = card.querySelector(".plan-feedback-input");
      if (execBtn) execBtn.addEventListener("click", () => runPlanAction("execute", mdEl?.value || null, null));
      if (revBtn) revBtn.addEventListener("click", () => runPlanAction("request_changes", mdEl?.value || null, fbEl?.value || ""));
    });
  }

  const qCard = document.getElementById("pending-question-card");
  if (sess && sess.pending_question) {
    if (qCard.dataset.qid !== sess.pending_question.id) {
      qCard.dataset.qid = sess.pending_question.id;
      selectedQuestionOpts.clear();
      qCard.classList.remove("hidden");
      document.getElementById("question-text").textContent = sess.pending_question.question;
      const optsEl = document.getElementById("question-options");
      optsEl.innerHTML = (sess.pending_question.options || [])
        .map((o) => `<button type="button" class="opt-chip" data-opt="${esc(o)}">${esc(o)}</button>`)
        .join("");
      optsEl.querySelectorAll(".opt-chip").forEach((chip) => {
        chip.addEventListener("click", () => {
          if (!sess.pending_question.allow_multiple) {
            selectedQuestionOpts.clear();
            optsEl.querySelectorAll(".opt-chip").forEach((c) => c.classList.remove("selected"));
          }
          if (selectedQuestionOpts.has(chip.dataset.opt)) {
            selectedQuestionOpts.delete(chip.dataset.opt);
            chip.classList.remove("selected");
          } else {
            selectedQuestionOpts.add(chip.dataset.opt);
            chip.classList.add("selected");
          }
        });
      });
    }
  } else {
    qCard.dataset.qid = "";
    qCard.classList.add("hidden");
  }

  const banner = document.getElementById("pending-approval-banner");
  if (sess && sess.pending_approval) {
    banner.classList.remove("hidden");
    const tier = sess.pending_approval.risk_tier || "workspace_mutation";
    banner.classList.toggle("risk-high", tier === "high_risk_destructive");
    const rBadge = document.getElementById("approval-risk-badge");
    rBadge.className = `badge ${tier === "high_risk_destructive" ? "badge-red" : tier === "safe_read_only" ? "badge-green" : "badge-amber"}`;
    rBadge.textContent = tier.replace(/_/g, " ").toUpperCase();
    document.getElementById("approval-title").textContent = sess.pending_approval.reason;
    const prev = document.getElementById("approval-preview");
    if (sess.pending_approval.diff_preview) prev.innerHTML = renderDiffHtml(sess.pending_approval.diff_preview);
    else prev.textContent = sess.pending_approval.arguments_json;
  } else {
    banner.classList.add("hidden");
  }
}

function wireCodeBlockButtons(root) {
  root.querySelectorAll(".cb-preview-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const snip = codeSnippetsStore.get(b.dataset.cbid);
      if (snip) openLightboxHtml(snip.code, "Interactive HTML Preview");
    });
  });
  root.querySelectorAll(".cb-copy-btn").forEach((b) => {
    b.addEventListener("click", async () => {
      const snip = codeSnippetsStore.get(b.dataset.cbid);
      if (snip) {
        await navigator.clipboard.writeText(snip.code).catch(() => {});
        showToast("Copied snippet to clipboard.");
      }
    });
  });
  root.querySelectorAll(".cb-apply-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const snip = codeSnippetsStore.get(b.dataset.cbid);
      if (!snip) return;
      switchView("editor");
      const tab = openTabs.find((t) => t.rel_path === activeTabPath);
      if (tab && (tab.kind === "text" || tab.kind === "markdown" || tab.kind === "html")) {
        tab.content = snip.code;
        tab.editing = true;
        renderEditorTabsAndPane();
        showToast(`Replaced draft content of ${tab.rel_path} (press Ctrl+S to save).`);
      } else {
        showToast("Open a text file in Code editor first to apply a snippet.", true);
      }
    });
  });
  root.querySelectorAll(".cb-run-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const snip = codeSnippetsStore.get(b.dataset.cbid);
      if (!snip) return;
      switchView("editor");
      document.getElementById("terminal-drawer").classList.remove("hidden");
      const firstLine = snip.code.trim().split("\n")[0];
      document.getElementById("terminal-cmd-input").value = firstLine;
      runTerminalCommand(firstLine);
    });
  });
}

function renderPlanCardHtml(plan) {
  const statusBadge =
    plan.status === "awaiting_review"
      ? `<span class="badge badge-amber">Awaiting Review</span>`
      : plan.status === "approved"
      ? `<span class="badge badge-blue">Approved</span>`
      : plan.status === "executed"
      ? `<span class="badge badge-green">Executed</span>`
      : `<span class="badge badge-purple">Changes Requested</span>`;

  const stepsHtml = (plan.steps || [])
    .map((s) => `<div class="plan-step-row">
      <div><strong>${s.step_number}.</strong> ${esc(s.description)}</div>
      ${s.files_touched?.length ? `<div class="muted-text mt-4">Files: ${s.files_touched.map((f) => `<code>${esc(f)}</code>`).join(", ")}</div>` : ""}
      ${s.verification ? `<div class="muted-text mt-4">Verify: <code>${esc(s.verification)}</code></div>` : ""}
    </div>`)
    .join("");

  const canAct = plan.status === "awaiting_review" || plan.status === "changes_requested";
  const actionControls = canAct
    ? `<details class="mt-6"><summary class="muted-text" style="cursor:pointer">Edit raw plan Markdown</summary>
        <textarea class="plan-md-editor">${esc(plan.markdown)}</textarea>
      </details>
      <div class="inline-row mt-6">
        <button type="button" class="btn-primary plan-exec-btn">Proceed &amp; Execute Plan</button>
        <input type="text" class="input-ctrl plan-feedback-input" placeholder="Feedback to revise plan..." />
        <button type="button" class="btn-small plan-revise-btn">Request Changes</button>
      </div>`
    : "";

  return `<div class="plan-card">
    <div class="card-title-row"><span>Plan Artifact: ${esc(plan.title)}</span>${statusBadge}</div>
    <div class="muted-text">${esc(plan.summary)}</div>
    <div class="plan-steps">${stepsHtml}</div>
    ${actionControls}
  </div>`;
}

function renderCheckpointInlineHtml(cp) {
  const filesHtml = cp.files
    .map((f) => {
      const stCls = f.status === "accepted" ? "badge-green" : f.status === "reverted" ? "badge-red" : "badge-amber";
      return `<div class="cp-file-row">
        <span><code>${esc(f.rel_path)}</code> <span class="badge ${stCls}">${esc(f.status)}</span></span>
        <div class="inline-row">
          ${f.status !== "accepted" ? `<button type="button" class="btn-small cp-accept-btn" data-cpid="${esc(cp.id)}" data-file="${esc(f.rel_path)}">Accept</button>` : ""}
          ${f.status !== "reverted" ? `<button type="button" class="btn-small cp-revert-btn" data-cpid="${esc(cp.id)}" data-file="${esc(f.rel_path)}">Revert</button>` : ""}
        </div>
      </div>`;
    })
    .join("");
  return `<div class="checkpoint-inline">
    <div class="card-title-row" style="margin-bottom:4px">
      <span>Checkpoint (${cp.files.length} file${cp.files.length === 1 ? "" : "s"})</span>
      <button type="button" class="btn-danger cp-rollback-btn" data-cpid="${esc(cp.id)}">Rollback Turn</button>
    </div>
    ${filesHtml}
  </div>`;
}

function wireCheckpointButtons(root) {
  root.querySelectorAll(".cp-accept-btn").forEach((b) => {
    b.addEventListener("click", () => checkpointFileAction(b.dataset.cpid, b.dataset.file, "accept"));
  });
  root.querySelectorAll(".cp-revert-btn").forEach((b) => {
    b.addEventListener("click", () => checkpointFileAction(b.dataset.cpid, b.dataset.file, "revert"));
  });
  root.querySelectorAll(".cp-rollback-btn").forEach((b) => {
    b.addEventListener("click", () => rollbackCheckpoint(b.dataset.cpid));
  });
}

function renderWorkspaceView() {
  setValIfIdle("rules-textarea", appState.workspace_rules || "");
  const grid = document.getElementById("workspace-cards-grid");
  grid.innerHTML = appState.workspaces
    .map((w) => {
      const active = w.path.toLowerCase() === appState.settings.active_workspace_path.toLowerCase();
      return `<button type="button" class="project-card ${active ? "active" : ""}" data-wspath="${esc(w.path)}">
        <div class="card-title-row"><span>${esc(w.name)}</span>${active ? '<span class="badge badge-amber">Active</span>' : ""}</div>
        <div class="muted-text" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(w.path)}</div>
      </button>`;
    })
    .join("");
  grid.querySelectorAll(".project-card").forEach((b) => {
    b.addEventListener("click", () => selectWorkspace(b.dataset.wspath));
  });
}

function renderFileTree() {
  const activeWs = appState.workspaces.find(
    (w) => w.path.toLowerCase() === appState.settings.active_workspace_path.toLowerCase()
  );
  document.getElementById("editor-ws-label").textContent = activeWs ? activeWs.name.toUpperCase() : "FILES";

  const q = (document.getElementById("tree-filter-input").value || "").trim().toLowerCase();
  const tree = document.getElementById("file-tree");
  const visible = (appState.workspace_tree || []).filter((n) => {
    const low = n.rel_path.toLowerCase();
    if (!q && (low === ".bumbo" || low.startsWith(".bumbo/") || low === ".cinder" || low.startsWith(".cinder/"))) {
      return false;
    }
    return !q || low.includes(q);
  });
  visible.sort((a, b) => {
    const ap = a.rel_path.includes("/") ? a.rel_path.slice(0, a.rel_path.lastIndexOf("/")) : "";
    const bp = b.rel_path.includes("/") ? b.rel_path.slice(0, b.rel_path.lastIndexOf("/")) : "";
    if (ap === bp && a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.rel_path.localeCompare(b.rel_path);
  });

  tree.innerHTML = visible
    .map((node) => {
      const depth = (node.rel_path.match(/\//g) || []).length;
      const pad = depth * 10 + 6;
      const icon = node.is_dir ? SVG.folder : SVG.file;
      const isOpen = !node.is_dir && activeTabPath && normPath(node.rel_path) === normPath(activeTabPath);
      return `<div class="tree-item ${node.is_dir ? "tree-dir" : ""} ${isOpen ? "active" : ""}" ${node.is_dir ? "" : 'draggable="true"'} style="padding-left:${pad}px" data-rel="${esc(node.rel_path)}" data-isdir="${node.is_dir}">
        <span class="inline-row" style="overflow:hidden;text-overflow:ellipsis">${icon} <span>${esc(node.name)}</span></span>
        ${node.is_dir ? "" : `<span class="muted-text">${fmtBytes(node.size_bytes)}</span>`}
      </div>`;
    })
    .join("");
  tree.querySelectorAll(".tree-item").forEach((el) => {
    if (el.dataset.isdir === "true") return;
    el.addEventListener("click", () => openWorkspaceFile(el.dataset.rel));
    el.addEventListener("dragstart", (e) => {
      e.dataTransfer.setData("application/x-bumbo-wspath", el.dataset.rel);
    });
  });
}

function syncEditorLineNumbers(text) {
  const count = Math.max(1, String(text || "").split("\n").length);
  let lines = "";
  for (let i = 1; i <= count; i++) lines += i + "\n";
  document.getElementById("editor-line-numbers").textContent = lines;
}

function updateActiveTabDirtyIndicator() {
  const tab = openTabs.find((t) => t.rel_path === activeTabPath);
  if (!tab) return;
  const dirty = tab.content !== tab.savedContent;
  const isEditable = tab.kind === "text" || tab.kind === "markdown" || tab.kind === "html";
  document.getElementById("editor-file-meta").textContent = `${tab.rel_path} · ${tab.kind.toUpperCase()} · ${fmtBytes(tab.size_bytes)}${dirty ? " · Unsaved changes" : ""}`;
  document.getElementById("save-file-btn").classList.toggle("hidden", !isEditable || (!tab.editing && !dirty));
  const tabEl = document.querySelector(`.ed-tab[data-tabpath="${CSS.escape(tab.rel_path)}"] > span`);
  if (tabEl) {
    const name = tab.rel_path.split("/").pop();
    tabEl.textContent = `${dirty ? "● " : ""}${name}`;
  }
}

function renderEditorTabsAndPane() {
  const tabsBar = document.getElementById("editor-tabs-bar");
  const metaEl = document.getElementById("editor-file-meta");
  const editBtn = document.getElementById("toggle-edit-btn");
  const saveBtn = document.getElementById("save-file-btn");
  const attachBtn = document.getElementById("attach-active-file-btn");
  const body = document.getElementById("preview-body");
  const wrap = document.getElementById("code-editor-wrap");
  const editor = document.getElementById("file-editor-textarea");

  if (!openTabs.length) {
    tabsBar.innerHTML = `<span class="muted-text" style="padding:0 8px">No open file</span>`;
    metaEl.textContent = "Click any file in the right-hand Workspace Explorer";
    editBtn.classList.add("hidden");
    saveBtn.classList.add("hidden");
    attachBtn.classList.add("hidden");
    wrap.classList.add("hidden");
    body.classList.remove("hidden");
    body.innerHTML = `<div class="empty-state">Select a file from the <strong>Workspace Explorer</strong> on the right (or drag files anywhere onto Cinder) to inspect PDFs, HTML, Markdown, images, or edit code with line numbers.</div>`;
    return;
  }

  tabsBar.innerHTML = openTabs
    .map((t) => {
      const dirty = t.content !== t.savedContent;
      const active = t.rel_path === activeTabPath ? "active" : "";
      const name = t.rel_path.split("/").pop();
      return `<div class="ed-tab ${active}" data-tabpath="${esc(t.rel_path)}">
        <span>${dirty ? "● " : ""}${esc(name)}</span>
        <button type="button" class="ed-tab-close" data-closepath="${esc(t.rel_path)}">×</button>
      </div>`;
    })
    .join("");
  tabsBar.querySelectorAll(".ed-tab").forEach((el) => {
    el.addEventListener("click", (e) => {
      if (e.target.classList.contains("ed-tab-close")) {
        const p = e.target.dataset.closepath;
        openTabs = openTabs.filter((x) => x.rel_path !== p);
        if (activeTabPath === p) activeTabPath = openTabs[0]?.rel_path || null;
        renderEditorTabsAndPane();
        renderFileTree();
        return;
      }
      activeTabPath = el.dataset.tabpath;
      renderEditorTabsAndPane();
      renderFileTree();
    });
  });

  const tab = openTabs.find((t) => t.rel_path === activeTabPath) || openTabs[0];
  activeTabPath = tab.rel_path;
  const dirty = tab.content !== tab.savedContent;
  metaEl.textContent = `${tab.rel_path} · ${tab.kind.toUpperCase()} · ${fmtBytes(tab.size_bytes)}${dirty ? " · Unsaved changes" : ""}`;

  const isEditable = tab.kind === "text" || tab.kind === "markdown" || tab.kind === "html";
  editBtn.classList.toggle("hidden", !isEditable);
  editBtn.textContent = tab.editing ? "Rich Preview" : "Edit Source";
  saveBtn.classList.toggle("hidden", !isEditable || (!tab.editing && !dirty));
  attachBtn.classList.remove("hidden");

  if (tab.editing && isEditable) {
    body.classList.add("hidden");
    wrap.classList.remove("hidden");
    if (document.activeElement !== editor && editor.value !== tab.content) {
      editor.value = tab.content || "";
    }
    syncEditorLineNumbers(editor.value);
    return;
  }

  wrap.classList.add("hidden");
  body.classList.remove("hidden");
  if (tab.kind === "image") {
    body.innerHTML = `<img class="preview-img" src="${tab.raw_url}" alt="${esc(tab.rel_path)}" />`;
  } else if (tab.kind === "html") {
    body.innerHTML = `<iframe class="preview-frame" sandbox="allow-same-origin" src="${tab.raw_url}"></iframe>`;
  } else if (tab.kind === "pdf") {
    body.innerHTML = `<iframe class="preview-frame" style="height:40vh" src="${tab.raw_url}"></iframe>
      <div class="card-title mt-6">Extracted Offline Text</div>
      <pre class="preview-code">${esc(tab.content)}</pre>`;
  } else if (tab.kind === "markdown") {
    body.innerHTML = `<div class="msg-body">${renderMarkdownLite(tab.content, "ed")}</div>`;
    wireCodeBlockButtons(body);
  } else {
    body.innerHTML = `<pre class="preview-code">${esc(tab.content)}</pre>`;
  }
}

function renderModulesView() {
  const enabled = new Set(appState.settings.ui?.enabled_modules || []);
  const grid = document.getElementById("modules-cards-grid");
  grid.innerHTML = HARNESS_MODULES.map((m) => {
    const on = enabled.has(m.id);
    return `<div class="module-card ${on ? "enabled" : ""}">
      <div class="card-title-row">
        <span>${esc(m.name)}</span>
        <input type="checkbox" class="mod-toggle-chk" data-modid="${esc(m.id)}" ${on ? "checked" : ""} />
      </div>
      <div class="muted-text">${esc(m.desc)}</div>
    </div>`;
  }).join("");

  const orch = appState.settings.orchestrator || {};
  const reachable = allReachableModels(true);
  const optHtml = (selId) =>
    reachable.map((m) => `<option value="${esc(m.id)}" ${m.id === selId ? "selected" : ""}>[${esc(m.providerLabel)}] ${esc(m.displayName)}</option>`).join("");
  document.getElementById("orch-planner-select").innerHTML = optHtml(orch.planner_model_id);
  document.getElementById("orch-executor-select").innerHTML = optHtml(orch.executor_model_id);
  setValIfIdle("orch-max-steps", orch.max_autonomous_steps || 8);
  document.getElementById("orch-review-gate").checked = !!orch.review_before_execute;
  document.getElementById("orch-auto-verify").checked = orch.auto_verify_after_edits !== false;
  document.getElementById("chk-auto-edits").checked = !!appState.settings.auto_approve_edits;
  document.getElementById("chk-auto-cmds").checked = !!appState.settings.auto_approve_commands;
}

function renderModelsRepoView() {
  const srv = appState.server_status;
  const badge = document.getElementById("server-badge");
  const stopBtn = document.getElementById("stop-server-btn");
  if (srv.running) {
    badge.className = "badge badge-green";
    badge.textContent = `${srv.backend === "llama_server" ? "LLAMA-SERVER" : "OLLAMA"} :${srv.port}`;
    stopBtn.classList.remove("hidden");
  } else {
    badge.className = "badge badge-muted";
    badge.textContent = "OFFLINE";
    stopBtn.classList.add("hidden");
  }

  const localEl = document.getElementById("local-models-full");
  const locals = appState.local_models.filter((m) => !m.is_embedding);
  if (!locals.length) {
    localEl.innerHTML = `<div class="muted-text">No GGUF models detected yet. Download one below or set your vault directory.</div>`;
  } else {
    localEl.innerHTML = locals
      .map((m) => {
        const isServing = srv.running && srv.model_id === m.id;
        const badgeCls = m.vram_fit.tier === "fits_in_vram" ? "badge-green" : m.vram_fit.tier === "partial_offload" ? "badge-amber" : "badge-red";
        return `<div class="model-card">
          <div class="card-title-row">
            <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(m.name)}</span>
            <button type="button" class="btn-small start-srv-btn" data-modelid="${esc(m.id)}">${isServing ? "Serving" : "Serve"}</button>
          </div>
          <div class="inline-row"><span class="badge ${badgeCls}">${esc(m.vram_fit.label)}</span><span class="muted-text">${fmtBytes(m.size_bytes)}</span></div>
        </div>`;
      })
      .join("");
    localEl.querySelectorAll(".start-srv-btn").forEach((b) => {
      b.addEventListener("click", () => startLocalServer(b.dataset.modelid));
    });
  }

  setValIfIdle("vault-dir-input", appState.settings.model_vault_dir);
  setValIfIdle("ctx-size-input", appState.settings.local_ctx_size);
  setValIfIdle("ngl-input", appState.settings.local_gpu_layers);
  setValIfIdle("idle-mins-input", appState.settings.idle_unload_minutes);

  document.getElementById("drives-list").innerHTML = appState.hardware.drives
    .map((d) => `<div class="drive-row">
      <span><strong>${esc(d.name)}</strong> ${fmtBytes(d.free_bytes)} free</span>
      <button type="button" class="btn-ghost use-drive-btn" data-mount="${esc(d.mount_point)}">Use</button>
    </div>`)
    .join("");
  document.getElementById("drives-list").querySelectorAll(".use-drive-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const m = b.dataset.mount;
      const sep = m.endsWith("\\") || m.endsWith("/") ? "" : "/";
      document.getElementById("vault-dir-input").value = `${m}${sep}Project_Bumbo_Models`;
    });
  });

  const eng = appState.engine_status;
  document.getElementById("engine-status-box").innerHTML = eng.llama_server_installed
    ? `<span class="badge badge-green">Installed</span> <code>${esc(eng.llama_server_path)}</code>`
    : `<span class="badge badge-amber">Not Installed</span> ${esc(eng.install_message)}${eng.ollama_loopback_detected ? ` · Ollama active (${eng.ollama_models.length} models)` : ""}`;

  // Complex 2026 Presets
  const presetsEl = document.getElementById("complex-presets-grid");
  if (presetModels.length) {
    presetsEl.innerHTML = presetModels
      .map((p) => {
        const cls = p.vram_fit.tier === "fits_in_vram" ? "badge-green" : "badge-amber";
        return `<div class="preset-card">
          <div class="card-title-row"><span>${esc(p.title)}</span><button type="button" class="btn-small inspect-preset-btn" data-repo="${esc(p.repo_id)}">Inspect GGUFs</button></div>
          <div class="muted-text">${esc(p.category)}</div>
          <div><span class="badge ${cls}">${esc(p.vram_fit.label)}</span></div>
        </div>`;
      })
      .join("");
    presetsEl.querySelectorAll(".inspect-preset-btn").forEach((b) => {
      b.addEventListener("click", () => inspectRepo(b.dataset.repo));
    });
  }

  // Keys & Auto-Switch
  const orch = appState.settings.orchestrator || {};
  document.getElementById("chk-only-reachable").checked = orch.only_show_reachable_models !== false;
  const chkAutoSw = document.getElementById("chk-auto-switch");
  if (chkAutoSw) chkAutoSw.checked = appState.settings.auto_switch_models !== false;
  const chkFbLocal = document.getElementById("chk-fallback-local");
  if (chkFbLocal) chkFbLocal.checked = !!appState.settings.fallback_to_local;
  const kf = appState.key_flags;
  document.getElementById("key-gemini").placeholder = kf.has_gemini_api_key ? "Gemini API Key (Configured)" : "Gemini API Key";
  document.getElementById("key-anthropic").placeholder = kf.has_anthropic_api_key ? "Anthropic API Key (Configured)" : "Anthropic API Key";
  document.getElementById("key-openrouter").placeholder = kf.has_openrouter_api_key ? "OpenRouter API Key (Configured)" : "OpenRouter API Key";
  document.getElementById("key-hf").placeholder = kf.has_huggingface_token ? "HF Token (Configured)" : "Hugging Face Token (optional)";
  document.getElementById("key-brave").placeholder = kf.has_brave_search_api_key ? "Brave Key (Configured)" : "Brave Search Key (optional)";
  setValIfIdle("key-oai-url", kf.openai_compat_base_url || "");
}

function renderProbeToolsHtml(serverId) {
  const probe = mcpProbeCache.get(serverId);
  if (!probe) return "";
  if (!probe.ok) {
    return `<div class="probe-tools-box" style="color:var(--red)">Probe failed: ${esc(probe.error || "Unknown error")}</div>`;
  }
  const tools = probe.tools || [];
  const pills = tools
    .map((t) => `<span class="badge badge-green" title="${esc(t.description)}">${esc(t.tool_name)}</span>`)
    .join("");
  return `<div class="probe-tools-box">
    <span class="badge badge-muted">${esc(probe.server_info || "MCP")} · ${probe.latency_ms}ms · ${tools.length} tool${tools.length === 1 ? "" : "s"}</span>
    ${pills || `<span class="muted-text">No tools advertised</span>`}
  </div>`;
}

async function probeMcpServerById(serverId) {
  showToast("Probing live MCP handshake...");
  try {
    const res = await api("/api/mcp/probe", {
      method: "POST",
      body: JSON.stringify({ id: serverId }),
    });
    if (res.probe) {
      mcpProbeCache.set(serverId, res.probe);
      renderMcpView();
      renderPluginsView();
      renderSettingsModal();
      if (res.probe.ok) {
        showToast(`MCP ready: ${res.probe.tools.length} tools (${res.probe.latency_ms}ms)`);
      } else {
        showToast(res.probe.error || "MCP probe failed", true);
      }
    }
  } catch (e) {
    showToast(e.message, true);
  }
}

async function toggleMcpServerEnabled(serverId, nextEnabled) {
  try {
    await api("/api/mcp/save", {
      method: "POST",
      body: JSON.stringify({ id: serverId, toggle_only: true, enabled: nextEnabled }),
    });
    await refreshState();
  } catch (e) {
    showToast(e.message, true);
  }
}

function ensureInstallJobsPolling() {
  const jobs = appState?.install_jobs || [];
  const hasRunning = jobs.some((j) => j.status === "running");
  if (hasRunning && !installJobsPollTimer) {
    installJobsPollTimer = setInterval(async () => {
      try {
        const res = await api("/api/install/jobs");
        const nextJobs = res.jobs || [];
        const wasRunning = (appState?.install_jobs || []).some((j) => j.status === "running");
        if (appState) appState.install_jobs = nextJobs;
        renderInstallJobs();
        const stillRunning = nextJobs.some((j) => j.status === "running");
        if (!stillRunning) {
          clearInterval(installJobsPollTimer);
          installJobsPollTimer = null;
          if (wasRunning) {
            await refreshState(true);
            const newest = nextJobs[0];
            if (newest?.status === "completed") {
              showToast(newest.summary || `Installed ${newest.target}.`);
            } else if (newest?.status === "failed") {
              showToast(newest.error || `Install failed for ${newest.target}.`, true);
            }
          }
        }
      } catch (_) {}
    }, 900);
  } else if (!hasRunning && installJobsPollTimer) {
    clearInterval(installJobsPollTimer);
    installJobsPollTimer = null;
  }
}

function renderInstallJobs() {
  const card = document.getElementById("install-jobs-card");
  const list = document.getElementById("install-jobs-list");
  if (!card || !list) return;
  const jobs = appState?.install_jobs || [];
  if (!jobs.length || (installJobsDismissed && !jobs.some((j) => j.status === "running"))) {
    card.classList.add("hidden");
    return;
  }
  card.classList.remove("hidden");
  list.innerHTML = jobs
    .slice(0, 6)
    .map((j) => {
      const badgeCls = j.status === "completed" ? "badge-green" : j.status === "failed" ? "badge-red" : "badge-amber";
      const fillCls = j.status === "completed" ? "meter-green" : j.status === "failed" ? "meter-red" : "meter-amber";
      const logTail = (j.logs || []).slice(-6).join("\n");
      return `<div class="install-job-card ${esc(j.status)}">
        <div class="card-title-row" style="margin-bottom:2px">
          <span><strong>${esc(j.target)}</strong> <span class="badge badge-muted">${esc(j.kind)}</span></span>
          <span class="badge ${badgeCls}">${esc(j.status)} · ${j.progress_pct}%</span>
        </div>
        <div class="meter-track"><div class="meter-fill ${fillCls}" style="width:${Math.max(4, j.progress_pct || 0)}%"></div></div>
        <div class="muted-text">${esc(j.current_step)}</div>
        ${logTail ? `<pre class="install-job-log">${esc(logTail)}</pre>` : ""}
      </div>`;
    })
    .join("");
}

async function triggerPluginInstall(pluginId) {
  installJobsDismissed = false;
  try {
    const res = await api("/api/plugins/install", {
      method: "POST",
      body: JSON.stringify({ plugin_id: pluginId }),
    });
    if (res.job && appState) {
      appState.install_jobs = [res.job, ...(appState.install_jobs || []).filter((j) => j.id !== res.job.id)];
      renderInstallJobs();
      ensureInstallJobsPolling();
    }
    showToast(`Started staged temp-folder install for ${pluginId}...`);
  } catch (e) {
    showToast(e.message, true);
  }
}

async function triggerPluginUninstall(pluginId) {
  try {
    await api("/api/plugins/uninstall", {
      method: "POST",
      body: JSON.stringify({ plugin_id: pluginId }),
    });
    mcpProbeCache.delete(`plugin::${pluginId}`);
    await refreshState(true);
    showToast(`Uninstalled ${pluginId}.`);
  } catch (e) {
    showToast(e.message, true);
  }
}

async function triggerDependencyInstall(manager, pkg) {
  installJobsDismissed = false;
  try {
    const res = await api("/api/dependencies/install", {
      method: "POST",
      body: JSON.stringify({ manager, package: pkg }),
    });
    if (res.job && appState) {
      appState.install_jobs = [res.job, ...(appState.install_jobs || []).filter((j) => j.id !== res.job.id)];
      renderInstallJobs();
      ensureInstallJobsPolling();
    }
    showToast(`Staging ${manager}:${pkg} in temporary folder...`);
  } catch (e) {
    showToast(e.message, true);
  }
}

async function syncAntigravityNow() {
  try {
    const res = await api("/api/antigravity/sync", {
      method: "POST",
      body: JSON.stringify({ include_builtin_skills: false }),
    });
    const rep = res.report || {};
    const nMcp = (rep.imported_mcp_servers || []).length;
    const nSk = (rep.imported_skills || []).length;
    await refreshState(true);
    showToast(`Antigravity sync complete: ${nMcp} MCP server(s), ${nSk} skill(s) imported.`);
  } catch (e) {
    showToast(e.message, true);
  }
}

async function exportAntigravityMcp() {
  try {
    const cfg = await api("/api/antigravity/export_mcp");
    document.getElementById("agy-export-pre").textContent = JSON.stringify(cfg, null, 2);
    document.getElementById("agy-export-card").classList.remove("hidden");
  } catch (e) {
    showToast(e.message, true);
  }
}

function renderPluginsView() {
  const strip = document.getElementById("toolchains-strip");
  const grid = document.getElementById("plugins-catalog-grid");
  const depsGrid = document.getElementById("deps-catalog-grid");
  if (!strip || !grid || !depsGrid || !appState) return;

  renderInstallJobs();
  ensureInstallJobsPolling();

  // 1. Managed Toolchains
  const tcs = appState.toolchains || [];
  strip.innerHTML = tcs
    .map((tc) => {
      const cls = tc.installed ? "ready" : "missing";
      const badge = tc.installed
        ? `<span class="badge badge-green">${esc(tc.source)}</span>`
        : `<span class="badge badge-amber">Missing</span>`;
      const instBtn = !tc.installed && tc.installable_plugin_id
        ? `<button type="button" class="btn-primary tc-install-btn mt-4" data-pid="${esc(tc.installable_plugin_id)}">Install Portable Node.js</button>`
        : "";
      return `<div class="toolchain-pill ${cls}">
        <div class="card-title-row" style="margin-bottom:0">
          <strong>${esc(tc.name)}</strong>
          ${badge}
        </div>
        <div class="muted-text" style="font-family:var(--font-mono);font-size:var(--fs-xs)">${esc(tc.version)}</div>
        ${instBtn}
      </div>`;
    })
    .join("");
  strip.querySelectorAll(".tc-install-btn").forEach((b) => {
    b.addEventListener("click", () => triggerPluginInstall(b.dataset.pid));
  });

  // 2. Curated Plugins & Antigravity Skills
  const allPlugins = appState.plugins_catalog || [];
  const filteredPlugins = allPlugins.filter((p) => {
    if (pluginCatFilter === "all") return true;
    if (pluginCatFilter === "mcp") return p.kind === "mcp_python" || p.kind === "mcp_node";
    return p.kind === pluginCatFilter;
  });
  const runningTargets = new Set(
    (appState.install_jobs || []).filter((j) => j.status === "running").map((j) => j.target)
  );

  grid.innerHTML = filteredPlugins
    .map((p) => {
      const isBusy = runningTargets.has(p.id);
      const netBadge = p.needs_network
        ? `<span class="badge badge-amber">Hybrid</span>`
        : `<span class="badge badge-green">Offline</span>`;
      const statusBadge = p.installed
        ? `<span class="badge badge-green">Installed</span>`
        : isBusy
        ? `<span class="badge badge-amber">Installing...</span>`
        : `<span class="badge badge-muted">v${esc(p.version)}</span>`;
      const probeHtml = p.mcp_server_id ? renderProbeToolsHtml(p.mcp_server_id) : "";

      let actions = "";
      if (p.installed) {
        if (p.mcp_server_id) {
          actions += `<button type="button" class="btn-small pl-probe-btn" data-mcpid="${esc(p.mcp_server_id)}">Test Tools</button>`;
          actions += `<button type="button" class="btn-ghost pl-toggle-btn" data-mcpid="${esc(p.mcp_server_id)}" data-enabled="${p.enabled ? "1" : "0"}">${p.enabled ? "Disable" : "Enable"}</button>`;
        }
        actions += `<button type="button" class="btn-danger pl-uninst-btn" data-pid="${esc(p.id)}">Uninstall</button>`;
      } else {
        actions += `<button type="button" class="btn-primary pl-inst-btn" data-pid="${esc(p.id)}" ${isBusy ? "disabled" : ""}>${isBusy ? "Staging..." : "Install (1-Click)"}</button>`;
      }

      return `<div class="plugin-card ${p.installed ? "installed" : ""}">
        <div class="card-title-row" style="margin-bottom:0">
          <strong>${esc(p.name)}</strong>
          ${statusBadge}
        </div>
        <div class="plugin-meta-row">
          <span class="badge badge-muted">${esc(p.category)}</span>
          ${netBadge}
          ${p.antigravity_compatible ? `<span class="badge badge-purple">Antigravity</span>` : ""}
        </div>
        <div class="muted-text">${esc(p.description)}</div>
        ${probeHtml}
        <div class="plugin-actions">${actions}</div>
      </div>`;
    })
    .join("");

  grid.querySelectorAll(".pl-inst-btn").forEach((b) => {
    b.addEventListener("click", () => triggerPluginInstall(b.dataset.pid));
  });
  grid.querySelectorAll(".pl-uninst-btn").forEach((b) => {
    b.addEventListener("click", () => triggerPluginUninstall(b.dataset.pid));
  });
  grid.querySelectorAll(".pl-probe-btn").forEach((b) => {
    b.addEventListener("click", () => probeMcpServerById(b.dataset.mcpid));
  });
  grid.querySelectorAll(".pl-toggle-btn").forEach((b) => {
    b.addEventListener("click", () => toggleMcpServerEnabled(b.dataset.mcpid, b.dataset.enabled !== "1"));
  });

  // 3. Curated Project Dependencies
  const depsInfo = appState.project_deps || { detected_ecosystems: [], installed_by_manager: {}, allowlist: [] };
  const detected = depsInfo.detected_ecosystems || [];
  if (!depMgrFilter) {
    depMgrFilter = detected[0] || "cargo";
  }
  document.querySelectorAll("#deps-mgr-tabs .cat-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.dmgr === depMgrFilter);
  });
  const wsName = (appState.settings.active_workspace_path || "").split(/[\\/]/).filter(Boolean).pop() || "workspace";
  document.getElementById("deps-workspace-badge").textContent = wsName;
  document.getElementById("deps-detected-note").innerHTML = detected.length
    ? `Detected project ecosystems: ${detected.map((e) => `<span class="badge badge-green">${esc(e)}</span>`).join(" ")} · Every install is verified in an isolated temp folder before touching your project.`
    : `No Cargo.toml, requirements.txt, or package.json detected in <code>${esc(wsName)}</code> yet — you can still stage Python or npm packages cleanly.`;

  const q = (document.getElementById("deps-search-input")?.value || "").trim().toLowerCase();
  const alreadyInstalled = new Set(
    ((depsInfo.installed_by_manager || {})[depMgrFilter] || []).map((x) => x.toLowerCase())
  );
  const matchingDeps = (depsInfo.allowlist || []).filter((d) => {
    if (d.manager !== depMgrFilter) return false;
    if (!q) return true;
    return `${d.name} ${d.category} ${d.description}`.toLowerCase().includes(q);
  });

  depsGrid.innerHTML = matchingDeps
    .map((d) => {
      const inProj = alreadyInstalled.has(d.name.toLowerCase());
      const busyKey = `${d.manager}:${d.name}`;
      const isBusy = runningTargets.has(busyKey);
      return `<div class="dep-card ${inProj ? "in-project" : ""}">
        <div class="card-title-row" style="margin-bottom:0">
          <code>${esc(d.name)}</code>
          ${inProj ? `<span class="badge badge-green">In Project</span>` : `<span class="badge badge-muted">${esc(d.category)}</span>`}
        </div>
        <div class="muted-text" style="font-size:var(--fs-xs)">${esc(d.description)}</div>
        <div class="plugin-actions">
          <button type="button" class="${inProj ? "btn-small" : "btn-primary"} dep-inst-btn" data-mgr="${esc(d.manager)}" data-pkg="${esc(d.name)}" ${isBusy ? "disabled" : ""}>
            ${isBusy ? "Staging..." : inProj ? "Reinstall / Update" : "+ Install"}
          </button>
        </div>
      </div>`;
    })
    .join("");

  depsGrid.querySelectorAll(".dep-inst-btn").forEach((b) => {
    b.addEventListener("click", () => triggerDependencyInstall(b.dataset.mgr, b.dataset.pkg));
  });
}

function renderMcpView() {
  document.getElementById("chk-expose-api").checked = !!appState.settings.expose_local_api;
  const srv = appState.server_status;
  document.getElementById("mcp-local-endpoint-info").innerHTML = srv.running
    ? `<span class="badge badge-green">Live Loopback</span> <code>${esc(srv.endpoint_url)}/v1/chat/completions</code> serving <strong>${esc(srv.model_name)}</strong>`
    : `Local endpoint offline. Serve any GGUF in Model Repository to bind <code>http://127.0.0.1:${appState.settings.local_server_port}/v1/chat/completions</code>.`;

  const listEl = document.getElementById("mcp-servers-list");
  const servers = appState.mcp_servers || [];
  if (!servers.length) {
    listEl.innerHTML = `<div class="muted-text">No external MCP servers configured yet. Install one with 1 click in <strong>Plugins &amp; Packages</strong> or sync from Antigravity.</div>`;
  } else {
    listEl.innerHTML = servers
      .map((s) => `<div class="model-card">
        <div class="card-title-row">
          <span>
            <strong>${esc(s.name)}</strong>
            <span class="badge badge-muted">${esc(s.transport)}</span>
            ${s.source ? `<span class="badge badge-purple">${esc(s.source)}</span>` : ""}
            <span class="badge ${s.enabled ? "badge-green" : "badge-muted"}">${s.enabled ? "Enabled" : "Disabled"}</span>
          </span>
          <div class="inline-row">
            <button type="button" class="btn-small probe-mcp-btn" data-mcpid="${esc(s.id)}">Test Tools</button>
            <button type="button" class="btn-ghost toggle-mcp-btn" data-mcpid="${esc(s.id)}" data-enabled="${s.enabled ? "1" : "0"}">${s.enabled ? "Disable" : "Enable"}</button>
            <button type="button" class="btn-ghost del-mcp-btn" data-mcpid="${esc(s.id)}">Remove</button>
          </div>
        </div>
        <div class="muted-text"><code>${esc(s.target)}</code></div>
        ${renderProbeToolsHtml(s.id)}
      </div>`)
      .join("");
    listEl.querySelectorAll(".probe-mcp-btn").forEach((b) => {
      b.addEventListener("click", () => probeMcpServerById(b.dataset.mcpid));
    });
    listEl.querySelectorAll(".toggle-mcp-btn").forEach((b) => {
      b.addEventListener("click", () => toggleMcpServerEnabled(b.dataset.mcpid, b.dataset.enabled !== "1"));
    });
    listEl.querySelectorAll(".del-mcp-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        await api("/api/mcp/save", {
          method: "POST",
          body: JSON.stringify({ id: b.dataset.mcpid, name: "", transport: "stdio", target: "", delete: true }),
        });
        mcpProbeCache.delete(b.dataset.mcpid);
        refreshState();
      });
    });
  }
}

function renderCodexView() {
  const skillsBox = document.getElementById("skills-list-box");
  skillsBox.innerHTML = (appState.skills || [])
    .map((s) => `<div class="model-card">
      <div class="card-title-row">
        <span><code>/${esc(s.slug)}</code> · ${esc(s.name)}</span>
        <span class="badge ${s.is_builtin ? "badge-muted" : "badge-amber"}">${s.recommended_mode}</span>
      </div>
      <div class="muted-text">${esc(s.description)}</div>
    </div>`)
    .join("");
}

function renderArtifactsTab() {
  const container = document.getElementById("artifacts-list");
  const sess = appState.active_session;
  const arts = sess?.artifacts || [];

  // Do not interrupt active text editing inside an artifact textarea
  if (document.activeElement && container.contains(document.activeElement)) return;

  if (!arts.length) {
    container.innerHTML = `<div class="empty-state">No session artifacts yet. Ask the agent to track tasks or click "+ New Note" above.</div>`;
    return;
  }

  for (const k of Array.from(codeSnippetsStore.keys())) {
    if (k.startsWith("cb_art")) codeSnippetsStore.delete(k);
  }

  container.innerHTML = arts
    .map((a, idx) => {
      const isEditing = artifactEditIds.has(a.id);
      const isHtml = a.kind === "html_widget";
      let bodyHtml = "";
      if (isEditing) {
        bodyHtml = `<textarea class="plan-md-editor art-edit-ta" data-artid="${esc(a.id)}">${esc(a.content)}</textarea>
          <div class="inline-row mt-4">
            <button type="button" class="btn-primary art-save-btn" data-artid="${esc(a.id)}" data-title="${esc(a.title)}" data-kind="${esc(a.kind)}">Save</button>
            <button type="button" class="btn-small art-cancel-btn" data-artid="${esc(a.id)}">Cancel</button>
          </div>`;
      } else if (isHtml) {
        const blobUrl = URL.createObjectURL(new Blob([a.content], { type: "text/html" }));
        bodyHtml = `<iframe class="artifact-html-frame" sandbox="allow-scripts" src="${blobUrl}"></iframe>`;
      } else {
        bodyHtml = `<div class="msg-body">${renderMarkdownLite(a.content, `art${idx}`)}</div>`;
      }

      return `<div class="artifact-card">
        <div class="card-title-row">
          <span>${esc(a.title)} <span class="badge badge-amber">${esc(a.kind)}</span></span>
          <span class="inline-row">
            <button type="button" class="btn-ghost art-edit-toggle-btn" data-artid="${esc(a.id)}">${isEditing ? "Preview" : "Edit"}</button>
            <button type="button" class="btn-ghost art-del-btn" data-artid="${esc(a.id)}">×</button>
          </span>
        </div>
        ${bodyHtml}
      </div>`;
    })
    .join("");

  wireCodeBlockButtons(container);

  container.querySelectorAll(".art-edit-toggle-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const id = b.dataset.artid;
      if (artifactEditIds.has(id)) artifactEditIds.delete(id);
      else artifactEditIds.add(id);
      renderArtifactsTab();
    });
  });
  container.querySelectorAll(".art-cancel-btn").forEach((b) => {
    b.addEventListener("click", () => {
      artifactEditIds.delete(b.dataset.artid);
      renderArtifactsTab();
    });
  });
  container.querySelectorAll(".art-save-btn").forEach((b) => {
    b.addEventListener("click", async () => {
      const id = b.dataset.artid;
      const ta = container.querySelector(`.art-edit-ta[data-artid="${CSS.escape(id)}"]`);
      const res = await api("/api/sessions/artifact", {
        method: "POST",
        body: JSON.stringify({
          session_id: appState.active_session.id,
          title: b.dataset.title,
          kind: b.dataset.kind,
          content: ta ? ta.value : "",
        }),
      });
      artifactEditIds.delete(id);
      applyTurnPayload(res);
      showToast("Saved artifact.");
    });
  });
  container.querySelectorAll(".art-del-btn").forEach((b) => {
    b.addEventListener("click", async () => {
      const res = await api("/api/sessions/artifact", {
        method: "POST",
        body: JSON.stringify({
          session_id: appState.active_session.id,
          artifact_id: b.dataset.artid,
          delete: true,
        }),
      });
      artifactEditIds.delete(b.dataset.artid);
      applyTurnPayload(res);
    });
  });
}

function renderTimelineDrawer() {
  renderArtifactsTab();
  document.getElementById("diff-mode-toggle-btn").textContent = `Side-by-Side: ${sideBySideDiff ? "On" : "Off"}`;
  const container = document.getElementById("changesets-container");
  const sess = appState.active_session;
  const cps = (sess?.checkpoints || []).slice().reverse();
  const hasPendingDiff = !!sess?.pending_approval?.diff_preview;

  if (!cps.length && !hasPendingDiff) {
    container.innerHTML = `<div class="empty-state">No checkpoints or diffs in this session yet.</div>`;
  } else {
    let html = "";
    if (hasPendingDiff) {
      html += `<div class="panel-card">
        <div class="card-title">Pending Review: ${esc(sess.pending_approval.target_path || sess.pending_approval.tool_name)}</div>
        <pre class="diff-pre">${renderDiffHtml(sess.pending_approval.diff_preview)}</pre>
      </div>`;
    }
    for (const cp of cps) {
      const filesHtml = cp.files
        .map((f) => {
          const stCls = f.status === "accepted" ? "badge-green" : f.status === "reverted" ? "badge-red" : "badge-amber";
          return `<div class="mt-6">
            <div class="card-title-row">
              <span><code>${esc(f.rel_path)}</code> <span class="badge ${stCls}">${esc(f.status)}</span></span>
              <div class="inline-row">
                ${f.status !== "accepted" ? `<button type="button" class="btn-small cp-accept-btn" data-cpid="${esc(cp.id)}" data-file="${esc(f.rel_path)}">Accept</button>` : ""}
                ${f.status !== "reverted" ? `<button type="button" class="btn-small cp-revert-btn" data-cpid="${esc(cp.id)}" data-file="${esc(f.rel_path)}">Revert</button>` : ""}
              </div>
            </div>
            <div class="diff-pre">${renderDiffHtml(f.diff)}</div>
          </div>`;
        })
        .join("");
      html += `<div class="panel-card">
        <div class="card-title-row"><span>${esc(cp.title)}</span><button type="button" class="btn-danger cp-rollback-btn" data-cpid="${esc(cp.id)}">Rollback Turn</button></div>
        ${filesHtml}
      </div>`;
    }
    container.innerHTML = html;
    wireCheckpointButtons(container);
  }

  document.getElementById("tier-select").value = appState.settings.quota_tier;
  document.getElementById("quota-cards").innerHTML = appState.cloud_quotas
    .map((q) => {
      const cls = q.remaining_daily_pct > 50 ? "meter-green" : q.remaining_daily_pct > 20 ? "meter-amber" : "meter-red";
      const hrs = Math.floor(q.reset_in_seconds / 3600);
      const mins = Math.floor((q.reset_in_seconds % 3600) / 60);
      const statusLabel = q.reachable
        ? "Reachable"
        : (q.reachability_reason || "").toLowerCase().includes("key")
        ? "Key Required"
        : "0 Quota / Unavail";
      return `<div class="quota-card">
        <div class="card-title-row">
          <span>${esc(q.antigravity_alias || q.display_name)} <span class="badge ${q.reachable ? "badge-green" : "badge-muted"}">${statusLabel}</span></span>
          <span><strong>${q.remaining_daily_pct.toFixed(1)}%</strong></span>
        </div>
        <div class="meter-track"><div class="meter-fill ${cls}" style="width:${q.remaining_daily_pct}%"></div></div>
        <div class="muted-text mt-4">RPM ${q.rpm_used}/${fmtNum(q.rpm_limit)} · RPD ${q.rpd_used}/${fmtNum(q.rpd_limit)} · Reset ${hrs}h ${mins}m</div>
      </div>`;
    })
    .join("");

  const audit = appState.privacy_audit;
  document.getElementById("audit-stats").innerHTML = `
    <span class="badge badge-green">Loopback: ${audit.total_loopback_requests}</span>
    <span class="badge badge-amber">External: ${audit.external_requests_allowed}</span>
    <span class="badge badge-red">Blocked: ${audit.external_requests_blocked}</span>
  `;
  const tableWrap = document.getElementById("audit-table-wrap");
  if (!audit.entries.length) {
    tableWrap.innerHTML = `<div class="muted-text">No network requests recorded yet.</div>`;
  } else {
    tableWrap.innerHTML = `<table class="audit-table">
      <thead><tr><th>Outcome</th><th>Host</th><th>Purpose</th></tr></thead>
      <tbody>${audit.entries.slice(0, 60).map((e) => `<tr><td>${esc(e.outcome)}</td><td>${esc(e.host)}</td><td>${esc(e.purpose)}</td></tr>`).join("")}</tbody>
    </table>`;
  }
}

function renderBumboDropdown(activeWs, audit, gpu, gpuLabel) {
  const srv = appState.server_status;
  const localCount = appState.local_models.filter((m) => !m.is_embedding).length;
  document.getElementById("bm-gpu-label").textContent = gpu ? gpuLabel : "CPU / System RAM";
  if (gpu && gpu.vram_total_mb > 0) {
    const pct = Math.min(100, (gpu.vram_used_mb / gpu.vram_total_mb) * 100);
    document.getElementById("bm-vram-label").textContent = `${gpu.vram_used_mb} / ${gpu.vram_total_mb} MB`;
    const bar = document.getElementById("bm-vram-bar");
    bar.style.width = `${pct.toFixed(1)}%`;
    bar.className = `meter-fill ${pct > 85 ? "meter-red" : pct > 60 ? "meter-amber" : "meter-green"}`;
  } else {
    document.getElementById("bm-vram-label").textContent = `${appState.hardware.system_ram_mb} MB RAM`;
    document.getElementById("bm-vram-bar").style.width = "10%";
  }

  document.getElementById("bm-engine-status").textContent = srv.running
    ? `Forging: ${srv.model_name || "Local GGUF"}`
    : `Forge cold · ${localCount} GGUFs`;
  document.getElementById("bm-unload-srv-btn").classList.toggle("hidden", !srv.running);

  const pBadge = document.getElementById("bm-privacy-badge");
  if (audit.privacy_mode === "strict_local") {
    pBadge.className = "badge badge-green";
    pBadge.textContent = "Strict Local (0 Leaks)";
  } else {
    pBadge.className = "badge badge-amber";
    pBadge.textContent = `Hybrid (${audit.external_requests_allowed} Ext)`;
  }

  const wsList = document.getElementById("bm-workspaces-list");
  wsList.innerHTML = appState.workspaces
    .map((w) => {
      const isAct = activeWs && w.path.toLowerCase() === activeWs.path.toLowerCase();
      return `<button type="button" class="bm-action-btn ${isAct ? "active" : ""}" data-bmws="${esc(w.path)}">
        <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(w.name)}</span>
        ${isAct ? '<span class="badge badge-amber">Active</span>' : ""}
      </button>`;
    })
    .join("");
  wsList.querySelectorAll("[data-bmws]").forEach((b) => {
    b.addEventListener("click", () => {
      closeAllPopovers();
      selectWorkspace(b.dataset.bmws);
    });
  });
}

function renderSettingsModal() {
  const ui = appState.settings.ui || {};
  document.getElementById("ui-theme-select").value = mapLegacyTheme(ui.theme);
  document.getElementById("ui-accent-select").value = mapLegacyAccent(ui.accent_color);
  document.getElementById("ui-scale-select").value = ui.font_scale || "default";
  document.getElementById("ui-wrap-code").checked = ui.wrap_code_blocks !== false;
  document.getElementById("ui-show-metrics").checked = ui.show_token_metrics !== false;
  setValIfIdle("ui-custom-sys", ui.custom_system_prompt || "");

  // Tab 2: Providers & Auto-Switch
  const orch = appState.settings.orchestrator || {};
  document.getElementById("sm-chk-only-reachable").checked = orch.only_show_reachable_models !== false;
  const smAutoSw = document.getElementById("sm-chk-auto-switch");
  if (smAutoSw) smAutoSw.checked = appState.settings.auto_switch_models !== false;
  const smFbLocal = document.getElementById("sm-chk-fallback-local");
  if (smFbLocal) smFbLocal.checked = !!appState.settings.fallback_to_local;
  const kf = appState.key_flags;
  document.getElementById("sm-key-gemini").placeholder = kf.has_gemini_api_key ? "Gemini API Key (Configured)" : "Gemini API Key";
  document.getElementById("sm-key-anthropic").placeholder = kf.has_anthropic_api_key ? "Anthropic API Key (Configured)" : "Anthropic API Key";
  document.getElementById("sm-key-openrouter").placeholder = kf.has_openrouter_api_key ? "OpenRouter API Key (Configured)" : "OpenRouter API Key";
  document.getElementById("sm-key-hf").placeholder = kf.has_huggingface_token ? "HF Token (Configured)" : "Hugging Face Token (optional)";
  document.getElementById("sm-key-brave").placeholder = kf.has_brave_search_api_key ? "Brave Key (Configured)" : "Brave Search Key (optional)";
  setValIfIdle("sm-key-oai-url", kf.openai_compat_base_url || "");

  // Tab 3: Modules & Orchestrator
  const enabled = new Set(ui.enabled_modules || []);
  document.getElementById("sm-modules-grid").innerHTML = HARNESS_MODULES.map((m) => {
    const on = enabled.has(m.id);
    return `<div class="module-card ${on ? "enabled" : ""}">
      <div class="card-title-row">
        <span>${esc(m.name)}</span>
        <input type="checkbox" class="sm-mod-chk" data-modid="${esc(m.id)}" ${on ? "checked" : ""} />
      </div>
      <div class="muted-text">${esc(m.desc)}</div>
    </div>`;
  }).join("");
  const reachable = allReachableModels(true);
  const optHtml = (selId) =>
    reachable.map((m) => `<option value="${esc(m.id)}" ${m.id === selId ? "selected" : ""}>[${esc(m.providerLabel)}] ${esc(m.displayName)}</option>`).join("");
  document.getElementById("sm-orch-planner").innerHTML = optHtml(orch.planner_model_id);
  document.getElementById("sm-orch-executor").innerHTML = optHtml(orch.executor_model_id);
  setValIfIdle("sm-orch-max-steps", orch.max_autonomous_steps || 8);
  document.getElementById("sm-orch-review-gate").checked = !!orch.review_before_execute;
  document.getElementById("sm-orch-auto-verify").checked = orch.auto_verify_after_edits !== false;
  document.getElementById("sm-chk-auto-edits").checked = !!appState.settings.auto_approve_edits;
  document.getElementById("sm-chk-auto-cmds").checked = !!appState.settings.auto_approve_commands;

  // Tab 4: Engine & Vault
  setValIfIdle("sm-vault-dir", appState.settings.model_vault_dir);
  setValIfIdle("sm-ctx-size", appState.settings.local_ctx_size);
  setValIfIdle("sm-ngl", appState.settings.local_gpu_layers);
  setValIfIdle("sm-idle-mins", appState.settings.idle_unload_minutes);
  document.getElementById("sm-unload-after-turn").checked = !!ui.unload_after_turn;
  document.getElementById("sm-expose-api").checked = !!appState.settings.expose_local_api;
  const eng = appState.engine_status;
  document.getElementById("sm-engine-status").innerHTML = eng.llama_server_installed
    ? `<span class="badge badge-green">Installed</span> <code>${esc(eng.llama_server_path)}</code>`
    : `<span class="badge badge-amber">Not Installed</span> ${esc(eng.install_message)}`;

  // Tab 5: Rules & MCP
  setValIfIdle("sm-rules-textarea", appState.workspace_rules || "");
  const smMcp = document.getElementById("sm-mcp-list");
  const servers = appState.mcp_servers || [];
  if (!servers.length) {
    smMcp.innerHTML = `<div class="muted-text">No external MCP servers configured yet.</div>`;
  } else {
    smMcp.innerHTML = servers
      .map((s) => `<div class="model-card">
        <div class="card-title-row">
          <span>${esc(s.name)} <span class="badge badge-muted">${esc(s.transport)}</span> <span class="badge ${s.enabled ? "badge-green" : "badge-muted"}">${s.enabled ? "On" : "Off"}</span></span>
          <div class="inline-row">
            <button type="button" class="btn-small sm-probe-mcp-btn" data-mcpid="${esc(s.id)}">Test</button>
            <button type="button" class="btn-ghost sm-del-mcp-btn" data-mcpid="${esc(s.id)}">Remove</button>
          </div>
        </div>
        <div class="muted-text"><code>${esc(s.target)}</code></div>
        ${renderProbeToolsHtml(s.id)}
      </div>`)
      .join("");
    smMcp.querySelectorAll(".sm-probe-mcp-btn").forEach((b) => {
      b.addEventListener("click", () => probeMcpServerById(b.dataset.mcpid));
    });
    smMcp.querySelectorAll(".sm-del-mcp-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        await api("/api/mcp/save", {
          method: "POST",
          body: JSON.stringify({ id: b.dataset.mcpid, name: "", transport: "stdio", target: "", delete: true }),
        });
        mcpProbeCache.delete(b.dataset.mcpid);
        refreshState();
      });
    });
  }
}

function renderCommandPalette(q) {
  if (!appState) return;
  const needle = q.trim().toLowerCase();
  const entries = [
    { kind: "View", label: "Home Dashboard", action: () => { forceHomeDashboard = true; switchView("home"); renderHomeStage(); } },
    { kind: "View", label: "Active Chat Stream", action: () => { forceHomeDashboard = false; switchView("home"); renderHomeStage(); } },
    { kind: "View", label: "Workspace & Projects", action: () => switchView("workspace") },
    { kind: "View", label: "Code Editor & Terminal", action: () => switchView("editor") },
    { kind: "View", label: "Modules & Policy", action: () => switchView("modules") },
    { kind: "View", label: "Model Repository", action: () => switchView("models") },
    { kind: "View", label: "Plugins, MCP Catalog & Project Dependencies", action: () => switchView("plugins") },
    { kind: "View", label: "MCP Server", action: () => switchView("mcp") },
    { kind: "View", label: "Codex Knowledge & Skills", action: () => switchView("codex") },
    { kind: "Action", label: "Sync Antigravity MCP & Skills (~/.gemini)", action: () => syncAntigravityNow() },
    { kind: "Action", label: "Export Antigravity mcp_config.json", action: () => { switchView("plugins"); exportAntigravityMcp(); } },
    { kind: "Action", label: "Compact Chat Context (/compact)", action: () => compactActiveSession() },
    { kind: "Action", label: "Export Session to Markdown (.md)", action: () => exportActiveSessionMd() },
    { kind: "Action", label: "Browse & Open Project Folder...", action: () => pickAndOpenWorkspaceFolder() },
    { kind: "Action", label: "Toggle Privacy Mode (Strict Local / Hybrid)", action: () => togglePrivacyMode() },
  ];

  for (const w of appState.workspaces) {
    entries.push({ kind: "Project", label: `${w.name} — ${w.path}`, action: () => selectWorkspace(w.path) });
  }
  for (const f of (appState.workspace_tree || []).filter((n) => !n.is_dir).slice(0, 80)) {
    entries.push({ kind: "File", label: f.rel_path, action: () => openWorkspaceFile(f.rel_path) });
  }
  for (const m of allReachableModels(true)) {
    entries.push({
      kind: "Model",
      label: m.displayName,
      action: async () => {
        await api("/api/settings/harness", { method: "POST", body: JSON.stringify({ active_model_id: m.id }) });
        await refreshState();
      },
    });
  }

  const matched = entries.filter((e) => !needle || `${e.kind} ${e.label}`.toLowerCase().includes(needle)).slice(0, 18);
  const box = document.getElementById("cmd-results");
  box.innerHTML = matched
    .map((e, i) => `<button type="button" class="cmd-item" data-cidx="${i}"><span>${esc(e.label)}</span><span class="badge badge-muted">${esc(e.kind)}</span></button>`)
    .join("");
  box.querySelectorAll(".cmd-item").forEach((b) => {
    b.addEventListener("click", () => {
      closeCommandPalette();
      matched[Number(b.dataset.cidx)].action();
    });
  });
}

function closeAllPopovers() {
  pickerState.a.open = false;
  pickerState.b.open = false;
  document.getElementById("model-picker-popover").classList.add("hidden");
  document.getElementById("model-b-picker-popover").classList.add("hidden");
  document.getElementById("attach-popover").classList.add("hidden");
  document.getElementById("slash-menu").classList.add("hidden");
  document.getElementById("at-file-menu").classList.add("hidden");
  document.getElementById("thinking-popover").classList.add("hidden");
  document.getElementById("downloads-popover").classList.add("hidden");
  document.getElementById("engine-guard-popover").classList.add("hidden");
  document.getElementById("bumbo-dropdown").classList.add("hidden");
}

function togglePicker(which) {
  const other = which === "a" ? "b" : "a";
  pickerState[other].open = false;
  document.getElementById(other === "a" ? "model-picker-popover" : "model-b-picker-popover").classList.add("hidden");
  pickerState[which].open = !pickerState[which].open;
  const pop = document.getElementById(which === "a" ? "model-picker-popover" : "model-b-picker-popover");
  pop.classList.toggle("hidden", !pickerState[which].open);
  if (pickerState[which].open) {
    renderModelPicker(which);
    const inp = document.getElementById(which === "a" ? "model-search-input" : "model-b-search-input");
    inp.focus();
  }
}

function renderAttachedChips() {
  const el = document.getElementById("attached-chips");
  const fileChips = attachedFiles.map(
    (f) => `<span class="file-chip">@${esc(f)} <button type="button" class="rm-chip-btn" data-file="${esc(f)}">×</button></span>`
  );
  const mediaChips = attachedMedia.map((m) => {
    if (m.mime_type?.startsWith("image/") && m.data_base64) {
      return `<span class="att-thumb-chip">
        <img class="att-thumb-img" src="data:${esc(m.mime_type)};base64,${m.data_base64}" alt="${esc(m.name)}" />
        <span>${esc(m.name)}</span>
        <button type="button" class="rm-media-btn" data-mid="${esc(m.id)}">×</button>
      </span>`;
    }
    return `<span class="file-chip">${esc(m.name)} (${fmtBytes(m.size_bytes)}) <button type="button" class="rm-media-btn" data-mid="${esc(m.id)}">×</button></span>`;
  });
  el.innerHTML = fileChips.concat(mediaChips).join("");
  el.querySelectorAll(".rm-chip-btn").forEach((b) => {
    b.addEventListener("click", () => {
      attachedFiles = attachedFiles.filter((x) => x !== b.dataset.file);
      renderAttachedChips();
    });
  });
  el.querySelectorAll(".rm-media-btn").forEach((b) => {
    b.addEventListener("click", () => {
      attachedMedia = attachedMedia.filter((x) => x.id !== b.dataset.mid);
      renderAttachedChips();
    });
  });
}

function readFileAsDataUrl(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ""));
    reader.onerror = () => reject(new Error(`Failed to read ${file.name}`));
    reader.readAsDataURL(file);
  });
}

async function uploadBrowserFile(file, saveToWorkspace = false) {
  if (!file) return;
  if (file.size > 16 * 1024 * 1024) {
    showToast(`${file.name} exceeds the 16 MB upload limit.`, true);
    return;
  }
  try {
    const dataUrl = await readFileAsDataUrl(file);
    const res = await api("/api/workspace/upload", {
      method: "POST",
      body: JSON.stringify({
        filename: file.name || `upload-${Date.now()}.png`,
        mime_type: file.type || null,
        data_base64: dataUrl,
        save_to_workspace: saveToWorkspace,
      }),
    });
    if (saveToWorkspace && res.rel_path) {
      await refreshState(true);
      await openWorkspaceFile(res.rel_path);
      showToast(`Saved & opened ${res.rel_path}`);
      return;
    }
    attachedMedia.push({
      id: res.id,
      name: res.name,
      mime_type: res.mime_type,
      data_base64: res.data_base64,
      text_content: res.text_content,
      size_bytes: res.size_bytes,
    });
    renderAttachedChips();
    if (currentView !== "home") switchView("home");
    showToast(`Attached ${res.name} (${res.is_image ? "Vision image" : "Extracted text"})`);
  } catch (e) {
    showToast(e.message, true);
  }
}

function startActivityPolling() {
  isGenerating = true;
  document.getElementById("send-icon").innerHTML = SVG.stopRect;
  document.getElementById("send-btn").classList.add("is-stop");
  document.getElementById("live-stream-card").classList.remove("hidden");
  document.getElementById("live-activity-text").textContent = "Starting...";
  document.getElementById("live-stream-tools").innerHTML = "";
  document.getElementById("live-stream-thinking").classList.add("hidden");
  document.getElementById("live-stream-content").classList.add("hidden");
  renderHomeStage();

  clearInterval(activityPollTimer);
  activityPollTimer = setInterval(async () => {
    try {
      const act = await api("/api/activity");
      if (act.active) {
        document.getElementById("live-activity-text").textContent = act.detail || `${act.phase} (${act.step}/${act.max_steps})`;
        const toolsBox = document.getElementById("live-stream-tools");
        toolsBox.innerHTML = (act.completed_tools || [])
          .map((t) => `<span class="badge ${t.status === "error" ? "badge-red" : "badge-green"}">${esc(t.name)} · ${esc(t.status)}</span>`)
          .join("");
        const thBox = document.getElementById("live-stream-thinking");
        if (act.partial_thinking?.trim()) {
          thBox.textContent = act.partial_thinking;
          thBox.classList.remove("hidden");
        }
        const cBox = document.getElementById("live-stream-content");
        if (act.partial_content?.trim()) {
          cBox.textContent = act.partial_content;
          cBox.classList.remove("hidden");
        }
      }
    } catch (_) {}
  }, 400);
}

function stopActivityPolling() {
  isGenerating = false;
  clearInterval(activityPollTimer);
  document.getElementById("send-icon").innerHTML = SVG.sendArrow;
  document.getElementById("send-btn").classList.remove("is-stop");
  document.getElementById("live-stream-card").classList.add("hidden");
}

async function sendOrAbortPrompt() {
  if (isGenerating) {
    await api("/api/chat/abort", { method: "POST" }).catch(() => {});
    showToast("Stopping generation...");
    return;
  }

  const input = document.getElementById("prompt-input");
  const text = input.value.trim();
  if ((!text && !attachedMedia.length && !attachedFiles.length) || !appState?.active_session) return;

  const finalMessage = text || "Please analyze the attached file(s)/image(s).";
  forceHomeDashboard = false;
  input.value = "";
  input.style.height = "auto";
  const filesCopy = [...attachedFiles];
  const mediaCopy = [...attachedMedia];
  attachedFiles = [];
  attachedMedia = [];
  renderAttachedChips();
  startActivityPolling();

  try {
    const mode = document.getElementById("mode-select").value;
    const reachableModels = allReachableModels(true);
    const chosenModelId =
      reachableModels.find((m) => m.id === appState.settings.active_model_id)?.id ||
      reachableModels[0]?.id ||
      appState.settings.active_model_id;
    const res = await api("/api/chat/send", {
      method: "POST",
      body: JSON.stringify({
        session_id: appState.active_session.id,
        message: finalMessage,
        mode,
        model_id: chosenModelId,
        compare_model_b_id: appState.settings.compare_model_b_id,
        attached_files: filesCopy,
        attachments: mediaCopy,
        thinking_effort: appState.settings.ui?.thinking_effort || "standard",
      }),
    });
    applyTurnPayload(res);
  } catch (e) {
    showToast(e.message, true);
    input.value = text;
    attachedFiles = filesCopy;
    attachedMedia = mediaCopy;
    renderAttachedChips();
  } finally {
    stopActivityPolling();
    renderHomeStage();
  }
}

async function rewindToMessage(idx) {
  if (!appState?.active_session) return;
  try {
    const res = await api("/api/chat/rewind", {
      method: "POST",
      body: JSON.stringify({ session_id: appState.active_session.id, message_index: idx, revert_files: true }),
    });
    attachedFiles = res.attached_files || [];
    attachedMedia = res.attachments || [];
    applyTurnPayload(res);
    const inp = document.getElementById("prompt-input");
    inp.value = res.prompt_text || "";
    inp.focus();
    showToast("Rewound conversation and restored file checkpoints.");
  } catch (e) {
    showToast(e.message, true);
  }
}

async function compactActiveSession() {
  if (!appState?.active_session) return;
  try {
    const res = await api("/api/chat/compact", {
      method: "POST",
      body: JSON.stringify({ session_id: appState.active_session.id }),
    });
    applyTurnPayload(res);
    showToast("Compacted older conversation messages into a summary.");
  } catch (e) {
    showToast(e.message, true);
  }
}

function exportActiveSessionMd() {
  const sess = appState?.active_session;
  if (!sess) return;
  const parts = [`# ${sess.title || "Cinder Session"}\n\nWorkspace: \`${sess.workspace_path}\`\nMode: \`${sess.mode}\`\n`];
  for (const m of sess.messages || []) {
    parts.push(`\n---\n### ${m.role === "user" ? "User" : (m.model_id || "Assistant")}\n\n${m.content}\n`);
  }
  if ((sess.artifacts || []).length) {
    parts.push(`\n---\n## Session Artifacts\n`);
    for (const a of sess.artifacts) {
      parts.push(`\n### ${a.title} (${a.kind})\n\n${a.content}\n`);
    }
  }
  const blob = new Blob([parts.join("\n")], { type: "text/markdown;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `${(sess.title || "cinder-session").replace(/[^a-z0-9_-]+/gi, "_")}.md`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

let isPickingFolder = false;
async function pickAndOpenWorkspaceFolder() {
  if (isPickingFolder) return;
  isPickingFolder = true;
  showToast("Opening system folder picker...");
  try {
    const res = await api("/api/system/pick_folder", { method: "POST" });
    if (res.picked && res.path) {
      await selectWorkspace(res.path);
    }
  } catch (e) {
    showToast(`${e.message} — Enter folder path manually in Projects & Rules.`, true);
    switchView("workspace");
    document.getElementById("custom-ws-input")?.focus();
  } finally {
    isPickingFolder = false;
  }
}

async function selectWorkspace(path) {
  try {
    attachedFiles = [];
    attachedMedia = [];
    openTabs = [];
    activeTabPath = null;
    depMgrFilter = null;
    await api("/api/workspace/select", { method: "POST", body: JSON.stringify({ path }) });
    await refreshState(true);
    showToast(`Workspace set to ${path}`);
  } catch (e) {
    showToast(e.message, true);
  }
}

async function openWorkspaceFile(relPath) {
  try {
    const existing = openTabs.find((t) => t.rel_path === relPath);
    if (!existing) {
      const file = await api(`/api/workspace/file?path=${encodeURIComponent(relPath)}`);
      openTabs.push({
        rel_path: file.rel_path,
        kind: file.kind,
        content: file.content || "",
        savedContent: file.content || "",
        raw_url: file.raw_url,
        size_bytes: file.size_bytes,
        editing: file.kind === "text",
      });
    }
    activeTabPath = relPath;
    switchView("editor");
    renderEditorTabsAndPane();
    renderFileTree();
  } catch (e) {
    showToast(e.message, true);
  }
}

async function saveActiveEditorTab() {
  const tab = openTabs.find((t) => t.rel_path === activeTabPath);
  if (!tab) return;
  try {
    const res = await api("/api/workspace/write_file", {
      method: "POST",
      body: JSON.stringify({ path: tab.rel_path, content: tab.content }),
    });
    tab.savedContent = tab.content;
    if (res.workspace_tree) appState.workspace_tree = res.workspace_tree;
    renderEditorTabsAndPane();
    renderFileTree();
    showToast(`Saved ${tab.rel_path}`);
  } catch (e) {
    showToast(e.message, true);
  }
}

async function runTerminalCommand(cmd) {
  const outEl = document.getElementById("terminal-output");
  outEl.textContent = `$ ${cmd}\nRunning...`;
  try {
    const res = await api("/api/workspace/terminal", { method: "POST", body: JSON.stringify({ command: cmd }) });
    const parts = [`$ ${cmd}  (exit ${res.exit_code}, ${res.duration_ms}ms)`];
    if (res.stdout?.trim()) parts.push(res.stdout.trim());
    if (res.stderr?.trim()) parts.push(`[STDERR]\n${res.stderr.trim()}`);
    outEl.textContent = parts.join("\n\n");
  } catch (e) {
    outEl.textContent = `Error: ${e.message}`;
  }
}

async function togglePrivacyMode() {
  const next = appState.privacy_audit.privacy_mode === "strict_local" ? "hybrid" : "strict_local";
  await api("/api/privacy", { method: "POST", body: JSON.stringify({ privacy_mode: next }) });
  await refreshState();
}

async function selectSession(sid) {
  forceHomeDashboard = false;
  await api("/api/sessions/select", { method: "POST", body: JSON.stringify({ session_id: sid }) });
  switchView("home");
  await refreshState();
}

async function deleteSession(sid) {
  await api(`/api/sessions/${encodeURIComponent(sid)}`, { method: "DELETE" });
  await refreshState();
}

async function runPlanAction(action, editedMarkdown, feedback) {
  if (!appState?.active_session) return;
  startActivityPolling();
  try {
    const res = await api("/api/chat/plan_action", {
      method: "POST",
      body: JSON.stringify({ session_id: appState.active_session.id, action, edited_markdown: editedMarkdown, feedback }),
    });
    applyTurnPayload(res);
  } catch (e) {
    showToast(e.message, true);
  } finally {
    stopActivityPolling();
    renderHomeStage();
  }
}

async function checkpointFileAction(checkpointId, relPath, action) {
  if (!appState?.active_session) return;
  try {
    const res = await api("/api/checkpoints/file_action", {
      method: "POST",
      body: JSON.stringify({ session_id: appState.active_session.id, checkpoint_id: checkpointId, rel_path: relPath, action }),
    });
    applyTurnPayload(res);
    showToast(action === "revert" ? `Reverted ${relPath}` : `Accepted ${relPath}`);
  } catch (e) {
    showToast(e.message, true);
  }
}

async function rollbackCheckpoint(checkpointId) {
  if (!appState?.active_session) return;
  try {
    const res = await api("/api/checkpoints/rollback", {
      method: "POST",
      body: JSON.stringify({ session_id: appState.active_session.id, checkpoint_id: checkpointId }),
    });
    applyTurnPayload(res);
    showToast("Rolled back turn checkpoint.");
  } catch (e) {
    showToast(e.message, true);
  }
}

async function startLocalServer(modelId) {
  try {
    showToast("Starting local model server in Strict Local Mode...");
    await api("/api/models/server/start", { method: "POST", body: JSON.stringify({ model_id: modelId }) });
    await refreshState(true);
    showToast("Local model active on 127.0.0.1.");
  } catch (e) {
    showToast(e.message, true);
  }
}

async function loadComplexPresets() {
  const res = await api("/api/models/presets");
  presetModels = res.presets || [];
  renderModelsRepoView();
}

async function searchHf() {
  const q = document.getElementById("hf-search-input").value.trim() || "Qwen2.5-Coder-14B-Instruct-GGUF";
  try {
    const res = await api(`/api/models/hf/search?q=${encodeURIComponent(q)}`);
    const box = document.getElementById("hf-results");
    document.getElementById("hf-repo-files").innerHTML = "";
    box.innerHTML = res.models
      .map((m) => `<div class="hf-row">
        <div class="card-title-row"><span>${esc(m.repo_id)}</span><button type="button" class="btn-small inspect-repo-btn" data-repo="${esc(m.repo_id)}">Inspect GGUFs</button></div>
        <div class="muted-text">Downloads: ${fmtNum(m.downloads)} · Likes: ${fmtNum(m.likes)}</div>
      </div>`)
      .join("");
    box.querySelectorAll(".inspect-repo-btn").forEach((b) => {
      b.addEventListener("click", () => inspectRepo(b.dataset.repo));
    });
  } catch (e) {
    showToast(e.message, true);
  }
}

async function inspectRepo(repoId) {
  try {
    selectedRepoId = repoId;
    const repo = await api(`/api/models/hf/repo?repo_id=${encodeURIComponent(repoId)}`);
    const box = document.getElementById("hf-repo-files");
    box.innerHTML =
      `<div class="card-title mt-6">${esc(repo.repo_id)} — GGUF &amp; mmproj Files</div>` +
      repo.files
        .map((f) => {
          const fitCls = f.vram_fit.tier === "fits_in_vram" ? "badge-green" : f.vram_fit.tier === "partial_offload" ? "badge-amber" : "badge-red";
          return `<div class="hf-row">
            <div class="card-title-row">
              <span>${esc(f.filename)} (${esc(f.quant)})</span>
              <button type="button" class="btn-small dl-hf-btn" data-file="${esc(f.filename)}" data-sha="${esc(f.sha256 || "")}" data-size="${f.size_bytes}">Download (${fmtBytes(f.size_bytes)})</button>
            </div>
            <div><span class="badge ${fitCls}">${esc(f.vram_fit.label)}</span></div>
          </div>`;
        })
        .join("");
    box.querySelectorAll(".dl-hf-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        try {
          await api("/api/models/hf/download", {
            method: "POST",
            body: JSON.stringify({
              repo_id: selectedRepoId,
              filename: b.dataset.file,
              sha256: b.dataset.sha || null,
              size_bytes: Number(b.dataset.size) || null,
            }),
          });
          showToast(`Started download: ${b.dataset.file}`);
          document.getElementById("downloads-popover").classList.remove("hidden");
          pollDownloads();
        } catch (e) {
          showToast(e.message, true);
        }
      });
    });
  } catch (e) {
    showToast(e.message, true);
  }
}

function pollDownloads() {
  clearInterval(downloadsPollTimer);
  downloadsPollTimer = setInterval(async () => {
    try {
      await refreshState();
      const anyActive = (appState?.downloads || []).some(
        (d) => d.status === "downloading" || d.status === "queued" || d.status === "verifying"
      );
      if (!anyActive) {
        clearInterval(downloadsPollTimer);
        await refreshState(true);
      }
    } catch (_) {
      clearInterval(downloadsPollTimer);
    }
  }, 1500);
}

function startWorkspaceWatcher() {
  clearInterval(watchPollTimer);
  watchPollTimer = setInterval(async () => {
    if (document.hidden) return;
    try {
      const w = await api("/api/workspace/watch");
      document.getElementById("watch-file-count").textContent = `${w.file_count} files`;
      if (lastWatchFingerprint && lastWatchFingerprint !== w.fingerprint) {
        await refreshState(true);
        for (const tab of openTabs) {
          if (tab.content === tab.savedContent) {
            const latest = await api(`/api/workspace/file?path=${encodeURIComponent(tab.rel_path)}`).catch(() => null);
            if (latest && latest.content !== undefined) {
              tab.content = latest.content;
              tab.savedContent = latest.content;
              tab.size_bytes = latest.size_bytes;
            }
          }
        }
        renderEditorTabsAndPane();
      }
      lastWatchFingerprint = w.fingerprint;
    } catch (_) {}
  }, 5000);
}

function renderPromptAutocompletes() {
  const inp = document.getElementById("prompt-input");
  const val = inp.value;
  const slashMenu = document.getElementById("slash-menu");
  const atMenu = document.getElementById("at-file-menu");

  if (val.startsWith("/") && !val.includes(" ")) {
    atMenu.classList.add("hidden");
    const needle = val.slice(1).toLowerCase();
    const matches = (appState?.skills || []).filter((s) => s.slug.toLowerCase().includes(needle));
    if (matches.length) {
      slashMenu.classList.remove("hidden");
      slashMenu.innerHTML = matches
        .map((s) => `<button type="button" class="slash-item" data-slug="${esc(s.slug)}">
          <span><strong>/${esc(s.slug)}</strong> <span class="muted-text">${esc(s.description)}</span></span>
          <span class="badge badge-muted">${esc(s.recommended_mode)}</span>
        </button>`)
        .join("");
      slashMenu.querySelectorAll(".slash-item").forEach((b) => {
        b.addEventListener("click", () => {
          inp.value = `/${b.dataset.slug} `;
          slashMenu.classList.add("hidden");
          inp.focus();
        });
      });
      return;
    }
  }
  slashMenu.classList.add("hidden");

  const atMatch = val.match(/(?:^|\s)@([^\s]*)$/);
  if (atMatch) {
    const q = atMatch[1].toLowerCase();
    const files = (appState?.workspace_tree || []).filter((n) => !n.is_dir && n.rel_path.toLowerCase().includes(q)).slice(0, 12);
    if (files.length) {
      atMenu.classList.remove("hidden");
      atMenu.innerHTML = files
        .map((f) => `<button type="button" class="slash-item" data-rel="${esc(f.rel_path)}"><span>@${esc(f.rel_path)}</span><span class="muted-text">${fmtBytes(f.size_bytes)}</span></button>`)
        .join("");
      atMenu.querySelectorAll(".slash-item").forEach((b) => {
        b.addEventListener("click", () => {
          const rel = b.dataset.rel;
          if (!attachedFiles.includes(rel)) {
            attachedFiles.push(rel);
            renderAttachedChips();
          }
          inp.value = val.replace(/@[^\s]*$/, "").trimEnd() + " ";
          atMenu.classList.add("hidden");
          inp.focus();
        });
      });
      return;
    }
  }
  atMenu.classList.add("hidden");
}

function wireModelPicker(which) {
  const btn = document.getElementById(which === "a" ? "model-picker-btn" : "model-b-picker-btn");
  const searchInp = document.getElementById(which === "a" ? "model-search-input" : "model-b-search-input");
  const catsEl = document.getElementById(which === "a" ? "model-cat-tabs" : "model-b-cat-tabs");

  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    togglePicker(which);
  });
  searchInp.addEventListener("input", () => {
    pickerState[which].query = searchInp.value;
    renderModelPicker(which);
  });
  catsEl.querySelectorAll(".cat-btn").forEach((c) => {
    c.addEventListener("click", () => {
      pickerState[which].cat = c.dataset.cat;
      catsEl.querySelectorAll(".cat-btn").forEach((x) => x.classList.toggle("active", x === c));
      renderModelPicker(which);
    });
  });
}

function collectKeyPayload() {
  return {
    gemini_api_key: document.getElementById("key-gemini").value || null,
    anthropic_api_key: document.getElementById("key-anthropic").value || null,
    openrouter_api_key: document.getElementById("key-openrouter").value || null,
    huggingface_token: document.getElementById("key-hf").value || null,
    brave_search_api_key: document.getElementById("key-brave").value || null,
    openai_compat_base_url: document.getElementById("key-oai-url").value,
    openai_compat_api_key: document.getElementById("key-oai-key").value || null,
  };
}

function renderAttachList() {
  const q = (document.getElementById("attach-search-input").value || "").trim().toLowerCase();
  const list = document.getElementById("attach-file-list");
  const files = (appState?.workspace_tree || []).filter((n) => !n.is_dir && (!q || n.rel_path.toLowerCase().includes(q)));
  list.innerHTML = files
    .slice(0, 40)
    .map((f) => `<button type="button" class="attach-file-opt" data-rel="${esc(f.rel_path)}">${esc(f.rel_path)}</button>`)
    .join("");
  list.querySelectorAll(".attach-file-opt").forEach((b) => {
    b.addEventListener("click", () => {
      if (!attachedFiles.includes(b.dataset.rel)) {
        attachedFiles.push(b.dataset.rel);
        renderAttachedChips();
      }
      document.getElementById("attach-popover").classList.add("hidden");
    });
  });
}

function wireDrawerResizer() {
  const expResizer = document.getElementById("drawer-resizer");
  const explorer = document.getElementById("right-explorer");
  const chatResizer = document.getElementById("chat-wb-resizer");
  const chatCol = document.getElementById("chat-column");
  const sidebar = document.getElementById("left-sidebar");
  let dragTarget = null;

  expResizer.addEventListener("mousedown", () => { dragTarget = "explorer"; });
  chatResizer.addEventListener("mousedown", () => { dragTarget = "chat"; });

  document.addEventListener("mousemove", (e) => {
    if (dragTarget === "explorer") {
      const width = Math.max(155, Math.min(420, window.innerWidth - e.clientX));
      explorer.style.width = `${width}px`;
      explorer.style.flex = `0 0 ${width}px`;
    } else if (dragTarget === "chat") {
      const root = document.getElementById("app-root");
      const sideW = root.classList.contains("sidebar-collapsed") ? 0 : sidebar.getBoundingClientRect().width;
      const expW = root.classList.contains("explorer-collapsed") ? 0 : explorer.getBoundingClientRect().width;
      const avail = Math.max(520, window.innerWidth - sideW - expW - 8);
      const chatW = Math.max(300, Math.min(avail - 300, e.clientX - sideW));
      chatCol.style.flex = `0 0 ${chatW}px`;
    }
  });
  document.addEventListener("mouseup", () => { dragTarget = null; });
}

function wireDragAndDrop() {
  const overlay = document.getElementById("drop-zone-overlay");
  const hasPayload = (dt) =>
    dt && (dt.types?.includes("Files") || dt.types?.includes("application/x-bumbo-wspath"));

  window.addEventListener("dragenter", (e) => {
    if (!hasPayload(e.dataTransfer)) return;
    e.preventDefault();
    dragEnterDepth++;
    overlay.classList.remove("hidden");
  });
  window.addEventListener("dragover", (e) => {
    if (!hasPayload(e.dataTransfer)) return;
    e.preventDefault();
  });
  window.addEventListener("dragleave", () => {
    dragEnterDepth = Math.max(0, dragEnterDepth - 1);
    if (dragEnterDepth === 0) overlay.classList.add("hidden");
  });
  window.addEventListener("drop", async (e) => {
    dragEnterDepth = 0;
    overlay.classList.add("hidden");
    if (!hasPayload(e.dataTransfer)) return;
    e.preventDefault();

    const wsRel = e.dataTransfer.getData("application/x-bumbo-wspath");
    if (wsRel) {
      if (!attachedFiles.includes(wsRel)) {
        attachedFiles.push(wsRel);
        renderAttachedChips();
      }
      if (currentView !== "home") switchView("home");
      document.getElementById("prompt-input").focus();
      showToast(`Attached @${wsRel}`);
      return;
    }

    const files = Array.from(e.dataTransfer.files || []).slice(0, 6);
    const saveToWs = currentView === "editor";
    for (const file of files) {
      await uploadBrowserFile(file, saveToWs);
    }
  });
}

async function stopAllLocalModelsNow() {
  closeAllPopovers();
  const res = await api("/api/models/server/stop", { method: "POST" });
  await refreshState(true);
  const unloaded = (res.unloaded || []).filter(Boolean);
  showToast(
    unloaded.length
      ? `Stopped local model(s) & freed VRAM: ${unloaded.join(", ")}`
      : "Local engine verified stopped (0 MB held in VRAM)."
  );
}

function wireEvents() {
  wireModelPicker("a");
  wireModelPicker("b");
  wireDrawerResizer();
  wireDragAndDrop();

  document.addEventListener("click", (e) => {
    if (
      !e.target.closest(".model-picker") &&
      !e.target.closest(".attach-anchor") &&
      !e.target.closest(".popover-anchor") &&
      !e.target.closest("#bumbo-dropdown") &&
      !e.target.closest("#brand-menu-btn") &&
      !e.target.closest("#top-bumbo-btn")
    ) {
      closeAllPopovers();
    }
  });

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      closeAllPopovers();
      closeSettingsModal();
      closeCommandPalette();
      closeLightbox();
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      openCommandPalette();
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "b") {
      e.preventDefault();
      document.getElementById("app-root").classList.toggle("sidebar-collapsed");
      syncPaneToggleStates();
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "e" && !e.shiftKey) {
      e.preventDefault();
      document.getElementById("app-root").classList.toggle("workbench-collapsed");
      syncPaneToggleStates();
    }
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "e") {
      e.preventDefault();
      document.getElementById("app-root").classList.toggle("explorer-collapsed");
      syncPaneToggleStates();
    }
    if ((e.ctrlKey || e.metaKey) && e.key === ",") {
      e.preventDefault();
      openSettingsModal();
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s" && currentView === "editor") {
      e.preventDefault();
      saveActiveEditorTab();
    }
    if (e.ctrlKey && e.key === "`") {
      e.preventDefault();
      switchView("editor");
      const term = document.getElementById("terminal-drawer");
      term.classList.toggle("hidden");
      if (!term.classList.contains("hidden")) {
        document.getElementById("terminal-cmd-input").focus();
      }
    }
  });

  // IDE 3-Pane Toggles, History, Menu Bar, Cinder Dropdown & Settings Triggers
  const toggleSidebar = () => {
    document.getElementById("app-root").classList.toggle("sidebar-collapsed");
    syncPaneToggleStates();
  };
  const toggleWorkbench = () => {
    document.getElementById("app-root").classList.toggle("workbench-collapsed");
    syncPaneToggleStates();
  };
  const toggleExplorer = () => {
    document.getElementById("app-root").classList.toggle("explorer-collapsed");
    syncPaneToggleStates();
  };
  document.getElementById("toggle-sidebar-btn").addEventListener("click", toggleSidebar);
  document.getElementById("sidebar-collapse-btn").addEventListener("click", toggleSidebar);
  document.getElementById("toggle-workbench-btn").addEventListener("click", toggleWorkbench);
  document.getElementById("wb-collapse-btn").addEventListener("click", toggleWorkbench);
  document.getElementById("toggle-explorer-btn").addEventListener("click", toggleExplorer);
  document.getElementById("explorer-close-btn").addEventListener("click", toggleExplorer);
  document.getElementById("explorer-refresh-btn").addEventListener("click", () => refreshState(true));
  document.getElementById("explorer-folder-btn").addEventListener("click", pickAndOpenWorkspaceFolder);

  document.getElementById("menu-workspace-btn").addEventListener("click", () => switchView("workspace"));
  document.getElementById("menu-view-btn").addEventListener("click", () => {
    const root = document.getElementById("app-root");
    if (root.classList.contains("workbench-collapsed")) root.classList.remove("workbench-collapsed");
    else root.classList.toggle("explorer-collapsed");
    syncPaneToggleStates();
  });
  document.getElementById("menu-window-btn").addEventListener("click", () => openSettingsModal());

  document.getElementById("nav-back-btn").addEventListener("click", () => navigateHistory(-1));
  document.getElementById("nav-fwd-btn").addEventListener("click", () => navigateHistory(1));

  const toggleBumboMenu = (e) => {
    e.stopPropagation();
    const dd = document.getElementById("bumbo-dropdown");
    const wasHidden = dd.classList.contains("hidden");
    closeAllPopovers();
    if (wasHidden) dd.classList.remove("hidden");
  };
  document.getElementById("brand-menu-btn").addEventListener("click", toggleBumboMenu);
  document.getElementById("top-bumbo-btn").addEventListener("click", toggleBumboMenu);

  document.getElementById("bm-unload-srv-btn").addEventListener("click", stopAllLocalModelsNow);
  document.getElementById("bm-privacy-toggle-btn").addEventListener("click", async () => {
    await togglePrivacyMode();
  });
  document.getElementById("bm-pick-folder-btn").addEventListener("click", () => {
    closeAllPopovers();
    pickAndOpenWorkspaceFolder();
  });
  document.getElementById("bm-goto-home-btn").addEventListener("click", () => {
    closeAllPopovers();
    forceHomeDashboard = true;
    switchView("home");
    renderHomeStage();
  });
  document.getElementById("bm-goto-chat-btn").addEventListener("click", () => {
    closeAllPopovers();
    forceHomeDashboard = false;
    switchView("home");
    renderHomeStage();
  });

  const startNewConversation = async () => {
    closeAllPopovers();
    forceHomeDashboard = false;
    if (appState?.active_session?.messages?.length) {
      await api("/api/sessions/new", { method: "POST", body: JSON.stringify({}) });
      await refreshState();
    }
    switchView("home");
    document.getElementById("prompt-input").focus();
  };
  document.getElementById("bm-new-chat-btn").addEventListener("click", startNewConversation);
  document.getElementById("bm-plugins-btn")?.addEventListener("click", () => {
    closeAllPopovers();
    switchView("plugins");
  });
  document.getElementById("new-chat-btn").addEventListener("click", startNewConversation);
  document.getElementById("sidebar-new-conv-btn").addEventListener("click", startNewConversation);

  document.getElementById("bm-compact-btn").addEventListener("click", () => {
    closeAllPopovers();
    compactActiveSession();
  });
  document.getElementById("bm-export-btn").addEventListener("click", () => {
    closeAllPopovers();
    exportActiveSessionMd();
  });
  document.getElementById("bm-rescan-btn").addEventListener("click", async () => {
    closeAllPopovers();
    await refreshState(true);
    showToast("Rescanned hardware telemetry, models, and workspace.");
  });
  document.getElementById("bm-settings-btn").addEventListener("click", () => {
    openSettingsModal();
  });

  document.getElementById("cmd-palette-btn").addEventListener("click", openCommandPalette);
  document.getElementById("cmd-search-input").addEventListener("input", (e) => renderCommandPalette(e.target.value));
  document.getElementById("cmd-modal").addEventListener("click", (e) => { if (e.target.id === "cmd-modal") closeCommandPalette(); });
  document.getElementById("close-lightbox-btn").addEventListener("click", closeLightbox);
  document.getElementById("image-lightbox-modal").addEventListener("click", (e) => { if (e.target.id === "image-lightbox-modal") closeLightbox(); });

  document.querySelectorAll(".nav-item[data-view]").forEach((b) => {
    b.addEventListener("click", () => switchView(b.dataset.view));
  });
  document.querySelectorAll(".wb-view-tab").forEach((b) => {
    b.addEventListener("click", () => switchView(b.dataset.wbview));
  });
  document.getElementById("nav-customizations").addEventListener("click", () => switchView("codex"));
  document.querySelectorAll(".cust-tab-btn").forEach((b) => {
    b.addEventListener("click", () => switchView(b.dataset.view));
  });
  document.getElementById("cust-back-chat-btn").addEventListener("click", () => {
    switchView("inspector");
    switchDrawerTab("artifacts");
  });
  document.getElementById("hdr-ws-pill").addEventListener("click", () => switchView("workspace"));

  document.getElementById("chat-review-btn").addEventListener("click", () => {
    switchView("inspector");
    switchDrawerTab("changesets");
  });
  document.getElementById("review-gate-pill-btn").addEventListener("click", async () => {
    const orch = appState.settings.orchestrator || {};
    const curAsk = !!orch.review_before_execute || !appState.settings.auto_approve_edits;
    const nextAsk = !curAsk;
    await api("/api/settings/customization", {
      method: "POST",
      body: JSON.stringify({ orchestrator: { ...orch, review_before_execute: nextAsk } }),
    });
    await api("/api/settings/harness", {
      method: "POST",
      body: JSON.stringify({ auto_approve_edits: !nextAsk }),
    });
    await refreshState();
    showToast(nextAsk ? "Request Review enabled (will pause before file edits/plans)." : "Request Review set to Auto.");
  });

  document.querySelectorAll(".quick-pill[data-view]").forEach((b) => {
    b.addEventListener("click", () => switchView(b.dataset.view));
  });
  document.getElementById("home-resume-chat-pill").addEventListener("click", () => {
    forceHomeDashboard = false;
    switchView("home");
    renderHomeStage();
  });
  document.getElementById("home-project-pill").addEventListener("click", pickAndOpenWorkspaceFolder);
  document.getElementById("home-browse-ws-btn").addEventListener("click", pickAndOpenWorkspaceFolder);
  document.getElementById("home-goto-models-btn").addEventListener("click", () => switchView("models"));
  document.getElementById("nav-new-project").addEventListener("click", pickAndOpenWorkspaceFolder);
  document.getElementById("ws-pick-folder-btn").addEventListener("click", pickAndOpenWorkspaceFolder);
  document.getElementById("browse-vault-btn").addEventListener("click", async () => {
    const res = await api("/api/system/pick_folder", { method: "POST" }).catch(() => ({}));
    if (res.picked && res.path) document.getElementById("vault-dir-input").value = res.path;
  });

  document.getElementById("toggle-session-filter-btn").addEventListener("click", () => {
    const wrap = document.getElementById("session-filter-wrap");
    wrap.classList.toggle("hidden");
    if (!wrap.classList.contains("hidden")) {
      document.getElementById("session-filter-input").focus();
    } else {
      document.getElementById("session-filter-input").value = "";
      renderTopAndSidebar();
    }
  });
  document.getElementById("session-filter-input").addEventListener("input", renderTopAndSidebar);
  document.getElementById("tree-filter-input").addEventListener("input", renderFileTree);
  document.getElementById("privacy-footer-btn").addEventListener("click", togglePrivacyMode);
  document.getElementById("open-settings-btn").addEventListener("click", () => openSettingsModal());
  document.getElementById("top-settings-btn").addEventListener("click", () => openSettingsModal());
  document.getElementById("close-settings-btn").addEventListener("click", closeSettingsModal);
  document.getElementById("settings-modal").addEventListener("click", (e) => { if (e.target.id === "settings-modal") closeSettingsModal(); });
  document.querySelectorAll(".settings-tab-btn").forEach((b) => {
    b.addEventListener("click", () => switchSettingsTab(b.dataset.stab));
  });

  // Topbar actions, Local Model VRAM Guard & popovers
  document.getElementById("compact-chat-btn").addEventListener("click", compactActiveSession);
  document.getElementById("export-chat-btn").addEventListener("click", exportActiveSessionMd);
  document.getElementById("engine-guard-btn").addEventListener("click", (e) => {
    e.stopPropagation();
    const pop = document.getElementById("engine-guard-popover");
    const wasHidden = pop.classList.contains("hidden");
    closeAllPopovers();
    if (wasHidden) pop.classList.remove("hidden");
  });
  document.getElementById("guard-refresh-btn").addEventListener("click", () => refreshState(true));
  document.getElementById("guard-stop-all-btn").addEventListener("click", stopAllLocalModelsNow);
  document.getElementById("chk-unload-after-turn").addEventListener("change", async (e) => {
    const ui = { ...appState.settings.ui, unload_after_turn: e.target.checked };
    await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ ui }) });
    await refreshState();
    showToast(e.target.checked ? "Auto-stop after every reply enabled." : "Auto-stop after every reply disabled.");
  });
  document.getElementById("guard-open-engine-btn").addEventListener("click", () => openSettingsModal("engine"));

  document.getElementById("downloads-btn").addEventListener("click", (e) => {
    e.stopPropagation();
    const pop = document.getElementById("downloads-popover");
    const wasHidden = pop.classList.contains("hidden");
    closeAllPopovers();
    if (wasHidden) pop.classList.remove("hidden");
  });
  document.getElementById("quick-setup-runtime-btn").addEventListener("click", async () => {
    try {
      showToast("Installing official llama.cpp CUDA 12 runtime...");
      const res = await api("/api/models/engine/install", { method: "POST", body: JSON.stringify({ backend: "cuda" }) });
      await refreshState(true);
      showToast(res.message);
    } catch (e) {
      showToast(e.message, true);
    }
  });
  document.getElementById("goto-models-btn").addEventListener("click", () => {
    closeAllPopovers();
    switchView("models");
  });
  document.getElementById("refresh-state-btn").addEventListener("click", () => refreshState(true));

  document.getElementById("timeline-btn").addEventListener("click", () => {
    switchView("inspector");
  });
  document.getElementById("quota-pill-btn").addEventListener("click", () => {
    switchView("inspector");
    switchDrawerTab("quotas");
  });

  document.querySelectorAll(".tab-btn").forEach((b) => {
    b.addEventListener("click", () => switchDrawerTab(b.dataset.tab));
  });
  document.getElementById("new-artifact-btn").addEventListener("click", async () => {
    if (!appState?.active_session) return;
    const res = await api("/api/sessions/artifact", {
      method: "POST",
      body: JSON.stringify({
        session_id: appState.active_session.id,
        title: `Task Notes ${((appState.active_session.artifacts || []).length + 1)}`,
        kind: "task_list",
        content: "- [ ] Item 1\n- [ ] Item 2",
      }),
    });
    if (res.artifact?.id) artifactEditIds.add(res.artifact.id);
    applyTurnPayload(res);
  });
  document.getElementById("diff-mode-toggle-btn").addEventListener("click", () => {
    sideBySideDiff = !sideBySideDiff;
    renderTimelineDrawer();
  });

  // Composer, Upload & Controls
  document.getElementById("upload-pc-file-btn").addEventListener("click", () => {
    document.getElementById("native-file-input").click();
  });
  document.getElementById("native-file-input").addEventListener("change", async (e) => {
    closeAllPopovers();
    const files = Array.from(e.target.files || []).slice(0, 6);
    for (const file of files) {
      await uploadBrowserFile(file, false);
    }
    e.target.value = "";
  });

  document.getElementById("thinking-pill-btn").addEventListener("click", (e) => {
    e.stopPropagation();
    document.getElementById("thinking-popover").classList.toggle("hidden");
  });
  document.querySelectorAll("#thinking-popover .pop-opt").forEach((b) => {
    b.addEventListener("click", async () => {
      const ui = { ...appState.settings.ui, thinking_effort: b.dataset.effort };
      document.getElementById("thinking-popover").classList.add("hidden");
      await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ ui }) });
      await refreshState();
    });
  });
  document.getElementById("composer-modules-btn").addEventListener("click", () => switchView("modules"));

  document.getElementById("mode-select").addEventListener("change", async (e) => {
    const newMode = e.target.value;
    if (appState?.active_session) appState.active_session.mode = newMode;
    renderComposerModeAndPickers(true);
    if (appState?.active_session?.id) {
      await api("/api/sessions/mode", {
        method: "POST",
        body: JSON.stringify({ session_id: appState.active_session.id, mode: newMode }),
      }).catch((err) => showToast(err.message, true));
    }
  });

  document.getElementById("attach-file-btn").addEventListener("click", (e) => {
    e.stopPropagation();
    const pop = document.getElementById("attach-popover");
    pop.classList.toggle("hidden");
    if (!pop.classList.contains("hidden")) {
      renderAttachList();
      document.getElementById("attach-search-input").focus();
    }
  });
  document.getElementById("attach-search-input").addEventListener("input", renderAttachList);

  document.getElementById("composer-form").addEventListener("submit", (e) => {
    e.preventDefault();
    sendOrAbortPrompt();
  });
  document.getElementById("live-abort-btn").addEventListener("click", () => {
    api("/api/chat/abort", { method: "POST" }).catch(() => {});
  });

  const promptEl = document.getElementById("prompt-input");
  promptEl.addEventListener("input", () => {
    promptEl.style.height = "auto";
    promptEl.style.height = `${Math.min(180, promptEl.scrollHeight)}px`;
    renderPromptAutocompletes();
  });
  promptEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendOrAbortPrompt();
    }
  });
  promptEl.addEventListener("paste", async (e) => {
    const items = Array.from(e.clipboardData?.items || []);
    const imgItem = items.find((it) => it.type && it.type.startsWith("image/"));
    if (imgItem) {
      const f = imgItem.getAsFile();
      if (f) {
        e.preventDefault();
        await uploadBrowserFile(f, false);
      }
    }
  });

  // Questions & Tool Approvals
  document.getElementById("submit-question-btn").addEventListener("click", async () => {
    startActivityPolling();
    try {
      const custom = document.getElementById("question-custom-input").value;
      const res = await api("/api/chat/answer_question", {
        method: "POST",
        body: JSON.stringify({ session_id: appState.active_session.id, selected_options: Array.from(selectedQuestionOpts), custom_text: custom }),
      });
      document.getElementById("question-custom-input").value = "";
      applyTurnPayload(res);
    } catch (e) { showToast(e.message, true); } finally { stopActivityPolling(); renderHomeStage(); }
  });
  document.getElementById("approve-tool-btn").addEventListener("click", async () => {
    startActivityPolling();
    try {
      const res = await api("/api/chat/approve_tool", { method: "POST", body: JSON.stringify({ session_id: appState.active_session.id, approve: true }) });
      applyTurnPayload(res);
    } catch (e) { showToast(e.message, true); } finally { stopActivityPolling(); renderHomeStage(); }
  });
  document.getElementById("reject-tool-btn").addEventListener("click", async () => {
    startActivityPolling();
    try {
      const res = await api("/api/chat/approve_tool", { method: "POST", body: JSON.stringify({ session_id: appState.active_session.id, approve: false }) });
      applyTurnPayload(res);
    } catch (e) { showToast(e.message, true); } finally { stopActivityPolling(); renderHomeStage(); }
  });

  // Workspace view
  document.getElementById("open-ws-btn").addEventListener("click", () => {
    const val = document.getElementById("custom-ws-input").value.trim();
    if (val) selectWorkspace(val);
  });
  document.getElementById("save-rules-btn").addEventListener("click", async () => {
    await api("/api/workspace/rules", { method: "POST", body: JSON.stringify({ content: document.getElementById("rules-textarea").value }) });
    showToast("Saved .cinder/rules.md");
  });

  // Code Editor & Terminal (Fast non-destructive input handling + Tab indent)
  const editorEl = document.getElementById("file-editor-textarea");
  editorEl.addEventListener("input", () => {
    const tab = openTabs.find((t) => t.rel_path === activeTabPath);
    if (tab) tab.content = editorEl.value;
    updateActiveTabDirtyIndicator();
    if (!rafLineSync) {
      rafLineSync = requestAnimationFrame(() => {
        rafLineSync = null;
        syncEditorLineNumbers(editorEl.value);
      });
    }
  });
  editorEl.addEventListener("keydown", (e) => {
    if (e.key === "Tab") {
      e.preventDefault();
      const s = editorEl.selectionStart;
      const end = editorEl.selectionEnd;
      editorEl.value = editorEl.value.slice(0, s) + "  " + editorEl.value.slice(end);
      editorEl.selectionStart = editorEl.selectionEnd = s + 2;
      editorEl.dispatchEvent(new Event("input"));
    }
  });
  editorEl.addEventListener("scroll", () => {
    document.getElementById("editor-line-numbers").scrollTop = editorEl.scrollTop;
  });
  document.getElementById("attach-active-file-btn").addEventListener("click", () => {
    if (activeTabPath && !attachedFiles.includes(activeTabPath)) {
      attachedFiles.push(activeTabPath);
    }
    forceHomeDashboard = false;
    switchView("home");
    renderAttachedChips();
    document.getElementById("prompt-input").focus();
  });
  document.getElementById("toggle-edit-btn").addEventListener("click", () => {
    const tab = openTabs.find((t) => t.rel_path === activeTabPath);
    if (tab) { tab.editing = !tab.editing; renderEditorTabsAndPane(); }
  });
  document.getElementById("save-file-btn").addEventListener("click", saveActiveEditorTab);
  document.getElementById("toggle-terminal-btn").addEventListener("click", () => {
    document.getElementById("terminal-drawer").classList.toggle("hidden");
  });
  document.getElementById("close-terminal-btn").addEventListener("click", () => {
    document.getElementById("terminal-drawer").classList.add("hidden");
  });
  document.getElementById("terminal-run-btn").addEventListener("click", () => {
    const cmd = document.getElementById("terminal-cmd-input").value.trim();
    if (cmd) runTerminalCommand(cmd);
  });
  document.getElementById("terminal-cmd-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      const cmd = e.target.value.trim();
      if (cmd) runTerminalCommand(cmd);
    }
  });
  document.querySelectorAll(".term-quick-btn").forEach((b) => {
    b.addEventListener("click", () => {
      document.getElementById("terminal-cmd-input").value = b.dataset.cmd;
      runTerminalCommand(b.dataset.cmd);
    });
  });
  document.getElementById("send-term-to-chat-btn").addEventListener("click", () => {
    const out = document.getElementById("terminal-output").textContent;
    const inp = document.getElementById("prompt-input");
    forceHomeDashboard = false;
    inp.value = `Please inspect this terminal output:\n\`\`\`\n${out.slice(0, 4000)}\n\`\`\`\n`;
    switchView("home");
    inp.focus();
  });

  // Modules & Policy
  document.getElementById("save-modules-btn").addEventListener("click", async () => {
    const enabledModules = Array.from(document.querySelectorAll(".mod-toggle-chk:checked")).map((c) => c.dataset.modid);
    const ui = { ...appState.settings.ui, enabled_modules: enabledModules };
    const orch = {
      ...appState.settings.orchestrator,
      planner_model_id: document.getElementById("orch-planner-select").value,
      executor_model_id: document.getElementById("orch-executor-select").value,
      max_autonomous_steps: Number(document.getElementById("orch-max-steps").value) || 8,
      review_before_execute: document.getElementById("orch-review-gate").checked,
      auto_verify_after_edits: document.getElementById("orch-auto-verify").checked,
    };
    await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ ui, orchestrator: orch }) });
    await api("/api/settings/harness", {
      method: "POST",
      body: JSON.stringify({
        auto_approve_edits: document.getElementById("chk-auto-edits").checked,
        auto_approve_commands: document.getElementById("chk-auto-cmds").checked,
      }),
    });
    await refreshState();
    showToast("Saved modules & orchestrator policy.");
  });

  // Models Repo
  document.getElementById("repo-refresh-btn").addEventListener("click", () => refreshState(true));
  document.getElementById("stop-server-btn").addEventListener("click", stopAllLocalModelsNow);
  document.getElementById("save-vault-btn").addEventListener("click", async () => {
    try {
      await api("/api/models/vault/set_dir", { method: "POST", body: JSON.stringify({ dir: document.getElementById("vault-dir-input").value }) });
      await refreshState(true);
      showToast("Model vault updated.");
    } catch (e) { showToast(e.message, true); }
  });
  const installEngine = async (backend) => {
    try {
      showToast(`Installing official llama.cpp (${backend})...`);
      const res = await api("/api/models/engine/install", { method: "POST", body: JSON.stringify({ backend }) });
      await refreshState(true);
      showToast(res.message);
    } catch (e) { showToast(e.message, true); }
  };
  document.getElementById("install-cuda-btn").addEventListener("click", () => installEngine("cuda"));
  document.getElementById("install-vulkan-btn").addEventListener("click", () => installEngine("vulkan"));
  document.getElementById("save-engine-cfg-btn").addEventListener("click", async () => {
    await api("/api/settings/harness", {
      method: "POST",
      body: JSON.stringify({
        local_ctx_size: Number(document.getElementById("ctx-size-input").value),
        local_gpu_layers: Number(document.getElementById("ngl-input").value),
        idle_unload_minutes: Number(document.getElementById("idle-mins-input").value),
      }),
    });
    await refreshState(true);
    showToast("Applied local engine config.");
  });
  document.getElementById("hf-search-btn").addEventListener("click", searchHf);
  document.getElementById("hf-search-input").addEventListener("keydown", (e) => { if (e.key === "Enter") searchHf(); });

  document.getElementById("chk-only-reachable").addEventListener("change", async (e) => {
    const orch = { ...appState.settings.orchestrator, only_show_reachable_models: e.target.checked };
    await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ orchestrator: orch }) });
    await refreshState();
  });
  document.getElementById("chk-auto-switch")?.addEventListener("change", async (e) => {
    await api("/api/settings/harness", { method: "POST", body: JSON.stringify({ auto_switch_models: e.target.checked }) });
    await refreshState();
    showToast(e.target.checked ? "Auto model switching enabled." : "Auto model switching disabled.");
  });
  document.getElementById("chk-fallback-local")?.addEventListener("change", async (e) => {
    await api("/api/settings/harness", { method: "POST", body: JSON.stringify({ fallback_to_local: e.target.checked }) });
    await refreshState();
  });
  document.querySelectorAll(".clear-key-btn").forEach((b) => {
    b.addEventListener("click", async () => {
      await api("/api/providers/keys", { method: "POST", body: JSON.stringify({ clear_keys: [b.dataset.key] }) });
      await refreshState();
      showToast(`Cleared ${b.dataset.key} key.`);
    });
  });
  document.getElementById("save-keys-btn").addEventListener("click", async () => {
    await api("/api/providers/keys", { method: "POST", body: JSON.stringify(collectKeyPayload()) });
    await refreshState();
    showToast("Saved keys offline.");
  });
  document.getElementById("verify-keys-btn").addEventListener("click", async () => {
    try {
      const res = await api("/api/providers/verify", { method: "POST", body: JSON.stringify(collectKeyPayload()) });
      document.getElementById("provider-verify-badges").innerHTML = (res.statuses || [])
        .map((s) => `<span class="badge ${s.verified ? "badge-green" : s.configured ? "badge-amber" : "badge-muted"}">${esc(s.provider)}: ${esc(s.message)}</span>`)
        .join("");
      await refreshState();
      showToast("Live-probed and updated reachable models.");
    } catch (e) { showToast(e.message, true); }
  });
  document.getElementById("add-custom-model-btn").addEventListener("click", async () => {
    try {
      const id = document.getElementById("custom-model-id-input").value.trim();
      const display_name = document.getElementById("custom-model-name-input").value.trim() || null;
      await api("/api/providers/custom_model", { method: "POST", body: JSON.stringify({ id, display_name }) });
      document.getElementById("custom-model-id-input").value = "";
      document.getElementById("custom-model-name-input").value = "";
      await refreshState();
      showToast(`Registered ${id}.`);
    } catch (e) { showToast(e.message, true); }
  });

  // MCP & Plugins Hub
  document.getElementById("mcp-open-plugins-btn")?.addEventListener("click", () => switchView("plugins"));
  document.getElementById("mcp-sync-agy-btn")?.addEventListener("click", syncAntigravityNow);
  document.getElementById("plugins-sync-agy-btn")?.addEventListener("click", syncAntigravityNow);
  document.getElementById("plugins-export-agy-btn")?.addEventListener("click", exportAntigravityMcp);
  document.getElementById("plugins-refresh-btn")?.addEventListener("click", () => refreshState(true));
  document.getElementById("install-jobs-hide-btn")?.addEventListener("click", () => {
    installJobsDismissed = true;
    document.getElementById("install-jobs-card").classList.add("hidden");
  });
  document.getElementById("agy-export-close-btn")?.addEventListener("click", () => {
    document.getElementById("agy-export-card").classList.add("hidden");
  });
  document.getElementById("agy-export-copy-btn")?.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(document.getElementById("agy-export-pre").textContent);
      showToast("Copied Antigravity mcp_config.json to clipboard.");
    } catch (_) {
      showToast("Select and copy the JSON above.", true);
    }
  });
  document.querySelectorAll("#plugins-cat-tabs .cat-btn").forEach((b) => {
    b.addEventListener("click", () => {
      pluginCatFilter = b.dataset.pcat;
      document.querySelectorAll("#plugins-cat-tabs .cat-btn").forEach((x) => x.classList.toggle("active", x === b));
      renderPluginsView();
    });
  });
  document.querySelectorAll("#deps-mgr-tabs .cat-btn").forEach((b) => {
    b.addEventListener("click", () => {
      depMgrFilter = b.dataset.dmgr;
      renderPluginsView();
    });
  });
  document.getElementById("deps-search-input")?.addEventListener("input", renderPluginsView);

  document.getElementById("chk-expose-api").addEventListener("change", async (e) => {
    await api("/api/settings/harness", { method: "POST", body: JSON.stringify({ expose_local_api: e.target.checked }) });
    await refreshState();
  });
  document.getElementById("mcp-add-btn").addEventListener("click", async () => {
    const name = document.getElementById("mcp-name-input").value.trim();
    const target = document.getElementById("mcp-target-input").value.trim();
    if (!name || !target) return showToast("Name and target required.", true);
    await api("/api/mcp/save", {
      method: "POST",
      body: JSON.stringify({ name, transport: document.getElementById("mcp-transport-select").value, target }),
    });
    document.getElementById("mcp-name-input").value = "";
    document.getElementById("mcp-target-input").value = "";
    await refreshState();
    showToast(`Added MCP server ${name}.`);
  });

  // Codex
  document.getElementById("index-kb-btn").addEventListener("click", async () => {
    try {
      const res = await api("/api/knowledge/index", { method: "POST" });
      const s = res.stats;
      document.getElementById("kb-stats-box").innerHTML = `<span class="badge badge-green">Indexed</span> ${s.total_files} files · ${s.total_chunks} chunks`;
      showToast(`Indexed ${s.total_chunks} chunks offline.`);
    } catch (e) { showToast(e.message, true); }
  });
  const runKbSearch = async () => {
    const q = document.getElementById("kb-search-input").value.trim();
    if (!q) return;
    try {
      const res = await api("/api/knowledge/search", { method: "POST", body: JSON.stringify({ query: q, top_k: 6 }) });
      const hits = res.hits || [];
      document.getElementById("kb-hits-list").innerHTML = hits.length
        ? hits.map((h) => `<div class="kb-hit-card"><div class="card-title-row"><span><code>${esc(h.rel_path)}</code> · ${esc(h.location)}</span><span class="badge badge-amber">${h.score}</span></div><div class="muted-text" style="white-space:pre-wrap">${esc(h.snippet)}</div></div>`).join("")
        : `<div class="muted-text">No matches found.</div>`;
    } catch (e) { showToast(e.message, true); }
  };
  document.getElementById("kb-search-btn").addEventListener("click", runKbSearch);
  document.getElementById("kb-search-input").addEventListener("keydown", (e) => { if (e.key === "Enter") runKbSearch(); });
  document.getElementById("save-skill-btn").addEventListener("click", async () => {
    try {
      const slug = document.getElementById("skill-slug-input").value.trim();
      await api("/api/skills/save", {
        method: "POST",
        body: JSON.stringify({
          slug,
          name: document.getElementById("skill-name-input").value.trim() || slug,
          description: document.getElementById("skill-desc-input").value.trim() || "Custom skill",
          prompt_template: document.getElementById("skill-prompt-input").value.trim(),
          recommended_mode: document.getElementById("skill-mode-select").value,
          is_builtin: false,
        }),
      });
      await refreshState();
      showToast("Saved custom skill.");
    } catch (e) { showToast(e.message, true); }
  });

  // Quotas, Audit & Settings Modal
  document.getElementById("tier-select").addEventListener("change", async (e) => {
    await api("/api/providers/tier", { method: "POST", body: JSON.stringify({ quota_tier: e.target.value }) });
    await refreshState();
  });
  document.getElementById("sync-google-btn").addEventListener("click", async () => {
    try {
      const res = await api("/api/providers/sync_google", { method: "POST" });
      await refreshState();
      showToast(`Live-probed ${res.synced_count} reachable Google models.`);
    } catch (e) { showToast(e.message, true); }
  });
  document.getElementById("clear-audit-btn").addEventListener("click", async () => {
    await api("/api/network/clear", { method: "POST" });
    await refreshState();
  });
  document.getElementById("save-ui-settings-btn").addEventListener("click", async () => {
    const ui = {
      ...appState.settings.ui,
      theme: document.getElementById("ui-theme-select").value,
      accent_color: document.getElementById("ui-accent-select").value,
      font_scale: document.getElementById("ui-scale-select").value,
      wrap_code_blocks: document.getElementById("ui-wrap-code").checked,
      show_token_metrics: document.getElementById("ui-show-metrics").checked,
      custom_system_prompt: document.getElementById("ui-custom-sys").value,
    };
    await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ ui }) });
    await refreshState();
    showToast("Saved appearance & instructions.");
  });

  // Full 5-Tab Settings Modal (Tabs 2–5)
  const collectSmKeyPayload = () => ({
    gemini_api_key: document.getElementById("sm-key-gemini").value || null,
    anthropic_api_key: document.getElementById("sm-key-anthropic").value || null,
    openrouter_api_key: document.getElementById("sm-key-openrouter").value || null,
    huggingface_token: document.getElementById("sm-key-hf").value || null,
    brave_search_api_key: document.getElementById("sm-key-brave").value || null,
    openai_compat_base_url: document.getElementById("sm-key-oai-url").value,
    openai_compat_api_key: document.getElementById("sm-key-oai-key").value || null,
  });
  document.getElementById("sm-chk-only-reachable").addEventListener("change", async (e) => {
    const orch = { ...appState.settings.orchestrator, only_show_reachable_models: e.target.checked };
    await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ orchestrator: orch }) });
    await refreshState();
  });
  document.getElementById("sm-chk-auto-switch")?.addEventListener("change", async (e) => {
    await api("/api/settings/harness", { method: "POST", body: JSON.stringify({ auto_switch_models: e.target.checked }) });
    await refreshState();
    showToast(e.target.checked ? "Auto model switching enabled." : "Auto model switching disabled.");
  });
  document.getElementById("sm-chk-fallback-local")?.addEventListener("change", async (e) => {
    await api("/api/settings/harness", { method: "POST", body: JSON.stringify({ fallback_to_local: e.target.checked }) });
    await refreshState();
  });
  document.querySelectorAll(".sm-clear-key-btn").forEach((b) => {
    b.addEventListener("click", async () => {
      await api("/api/providers/keys", { method: "POST", body: JSON.stringify({ clear_keys: [b.dataset.key] }) });
      await refreshState();
      showToast(`Cleared ${b.dataset.key} key.`);
    });
  });
  document.getElementById("sm-save-keys-btn").addEventListener("click", async () => {
    await api("/api/providers/keys", { method: "POST", body: JSON.stringify(collectSmKeyPayload()) });
    await refreshState();
    showToast("Saved keys offline.");
  });
  document.getElementById("sm-verify-keys-btn").addEventListener("click", async () => {
    try {
      const res = await api("/api/providers/verify", { method: "POST", body: JSON.stringify(collectSmKeyPayload()) });
      document.getElementById("sm-provider-badges").innerHTML = (res.statuses || [])
        .map((s) => `<span class="badge ${s.verified ? "badge-green" : s.configured ? "badge-amber" : "badge-muted"}">${esc(s.provider)}: ${esc(s.message)}</span>`)
        .join("");
      await refreshState();
      showToast("Live-probed and updated reachable models.");
    } catch (e) { showToast(e.message, true); }
  });
  document.getElementById("sm-add-custom-model-btn").addEventListener("click", async () => {
    try {
      const id = document.getElementById("sm-custom-model-id").value.trim();
      const display_name = document.getElementById("sm-custom-model-name").value.trim() || null;
      await api("/api/providers/custom_model", { method: "POST", body: JSON.stringify({ id, display_name }) });
      document.getElementById("sm-custom-model-id").value = "";
      document.getElementById("sm-custom-model-name").value = "";
      await refreshState();
      showToast(`Registered ${id}.`);
    } catch (e) { showToast(e.message, true); }
  });

  document.getElementById("sm-save-autonomy-btn").addEventListener("click", async () => {
    const enabledModules = Array.from(document.querySelectorAll(".sm-mod-chk:checked")).map((c) => c.dataset.modid);
    const ui = { ...appState.settings.ui, enabled_modules: enabledModules };
    const orch = {
      ...appState.settings.orchestrator,
      planner_model_id: document.getElementById("sm-orch-planner").value,
      executor_model_id: document.getElementById("sm-orch-executor").value,
      max_autonomous_steps: Number(document.getElementById("sm-orch-max-steps").value) || 8,
      review_before_execute: document.getElementById("sm-orch-review-gate").checked,
      auto_verify_after_edits: document.getElementById("sm-orch-auto-verify").checked,
    };
    await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ ui, orchestrator: orch }) });
    await api("/api/settings/harness", {
      method: "POST",
      body: JSON.stringify({
        auto_approve_edits: document.getElementById("sm-chk-auto-edits").checked,
        auto_approve_commands: document.getElementById("sm-chk-auto-cmds").checked,
      }),
    });
    await refreshState();
    showToast("Saved modules & orchestrator policy.");
  });

  document.getElementById("sm-browse-vault-btn").addEventListener("click", async () => {
    const res = await api("/api/system/pick_folder", { method: "POST" }).catch(() => ({}));
    if (res.picked && res.path) document.getElementById("sm-vault-dir").value = res.path;
  });
  document.getElementById("sm-save-vault-btn").addEventListener("click", async () => {
    try {
      await api("/api/models/vault/set_dir", { method: "POST", body: JSON.stringify({ dir: document.getElementById("sm-vault-dir").value }) });
      await refreshState(true);
      showToast("Model vault updated.");
    } catch (e) { showToast(e.message, true); }
  });
  document.getElementById("sm-save-engine-btn").addEventListener("click", async () => {
    const ui = { ...appState.settings.ui, unload_after_turn: document.getElementById("sm-unload-after-turn").checked };
    await api("/api/settings/customization", { method: "POST", body: JSON.stringify({ ui }) });
    await api("/api/settings/harness", {
      method: "POST",
      body: JSON.stringify({
        local_ctx_size: Number(document.getElementById("sm-ctx-size").value),
        local_gpu_layers: Number(document.getElementById("sm-ngl").value),
        idle_unload_minutes: Number(document.getElementById("sm-idle-mins").value),
        expose_local_api: document.getElementById("sm-expose-api").checked,
      }),
    });
    await refreshState(true);
    showToast("Applied local engine settings.");
  });
  document.getElementById("sm-install-cuda-btn").addEventListener("click", () => installEngine("cuda"));
  document.getElementById("sm-install-vulkan-btn").addEventListener("click", () => installEngine("vulkan"));

  document.getElementById("sm-save-rules-btn").addEventListener("click", async () => {
    await api("/api/workspace/rules", { method: "POST", body: JSON.stringify({ content: document.getElementById("sm-rules-textarea").value }) });
    await refreshState();
    showToast("Saved .cinder/rules.md");
  });
  document.getElementById("sm-mcp-add-btn").addEventListener("click", async () => {
    const name = document.getElementById("sm-mcp-name").value.trim();
    const target = document.getElementById("sm-mcp-target").value.trim();
    if (!name || !target) return showToast("Name and target required.", true);
    await api("/api/mcp/save", {
      method: "POST",
      body: JSON.stringify({ name, transport: document.getElementById("sm-mcp-transport").value, target }),
    });
    document.getElementById("sm-mcp-name").value = "";
    document.getElementById("sm-mcp-target").value = "";
    await refreshState();
    showToast(`Added MCP server ${name}.`);
  });
}

document.addEventListener("DOMContentLoaded", () => {
  const root = document.getElementById("app-root");
  if (window.innerWidth <= 1080) {
    root.classList.add("sidebar-collapsed");
    root.classList.add("explorer-collapsed");
  }
  if (window.innerWidth <= 820) {
    root.classList.add("workbench-collapsed");
  }
  wireEvents();
  refreshState()
    .then(() => {
      switchView("inspector");
      if (window.innerWidth <= 820) {
        root.classList.add("workbench-collapsed");
        syncPaneToggleStates();
      }
      startWorkspaceWatcher();
      loadComplexPresets().catch(() => {});
    })
    .catch((e) => showToast(e.message, true));
});
