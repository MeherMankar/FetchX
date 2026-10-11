import "./popup.css";
import { PROTOCOL_VERSION, isError, userMessageForCode } from "../types/protocol";
import { NativeHostError, hasEmbeddedCredentials, isHttpUrl, sendToNativeHost } from "../shared/native";
import type { ExtensionSettings } from "../background/service-worker";

// ── DOM refs ──────────────────────────────────────────────────────────────

const statusBadge  = q<HTMLElement>("#status-badge");
const statusText   = q<HTMLElement>("#status-text");
const urlInput     = q<HTMLInputElement>("#url-input");
const useTabBtn    = q<HTMLButtonElement>("#use-tab-btn");
const submitBtn    = q<HTMLButtonElement>("#submit-btn");
const btnText      = q<HTMLElement>("#btn-text");
const resultBanner = q<HTMLElement>("#result-banner");
const taskList     = q<HTMLElement>("#task-list");
const refreshBtn   = q<HTMLButtonElement>("#refresh-tasks-btn");
const scanBtn      = q<HTMLButtonElement>("#scan-btn");
const scanEmpty    = q<HTMLElement>("#scan-empty");
const resourceList = q<HTMLElement>("#resource-list");
const resActions   = q<HTMLElement>("#resource-actions");
const selectedCount = q<HTMLElement>("#selected-count");
const dlSelectedBtn = q<HTMLButtonElement>("#download-selected-btn");
const settingsBtn  = q<HTMLButtonElement>("#settings-btn");
const settingsPanel = q<HTMLElement>("#settings-panel");
const settingsClose = q<HTMLButtonElement>("#settings-close-btn");
const settingsSave = q<HTMLButtonElement>("#settings-save-btn");
const setAutoIntercept = q<HTMLInputElement>("#set-auto-intercept");
const setMinSize   = q<HTMLInputElement>("#set-min-size");
const setNotif     = q<HTMLInputElement>("#set-notifications");

function q<T extends Element>(sel: string): T {
  return document.querySelector<T>(sel)!;
}

// ── Tabs ──────────────────────────────────────────────────────────────────

document.querySelectorAll<HTMLButtonElement>(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("tab-active"));
    document.querySelectorAll<HTMLElement>(".tab-panel").forEach((p) => p.classList.add("hidden"));
    btn.classList.add("tab-active");
    const panel = document.getElementById(`tab-${btn.dataset.tab}`);
    panel?.classList.remove("hidden");
    if (btn.dataset.tab === "tasks") void loadTasks();
  });
});

// ── Status ────────────────────────────────────────────────────────────────

function setStatus(state: "checking" | "connected" | "disconnected", label?: string) {
  statusBadge.className = `status-badge status-${state}`;
  statusText.textContent = label ?? { checking: "Checking…", connected: "Connected", disconnected: "Not running" }[state];
}

// ── Result banner ─────────────────────────────────────────────────────────

function showResult(kind: "success" | "error", msg: string) {
  resultBanner.textContent = msg;
  resultBanner.className = `result-banner ${kind}`;
}
function clearResult() {
  resultBanner.className = "result-banner hidden";
  resultBanner.textContent = "";
}

// ── URL / Add tab ─────────────────────────────────────────────────────────

function validateUrl(url: string): string | null {
  if (!url.trim()) return "Please enter a URL.";
  if (!isHttpUrl(url)) return "Only HTTP and HTTPS URLs are supported.";
  if (hasEmbeddedCredentials(url)) return "URLs with credentials are not accepted.";
  return null;
}

function updateSubmit() {
  submitBtn.disabled = validateUrl(urlInput.value.trim()) !== null;
}

urlInput.addEventListener("input", () => { clearResult(); updateSubmit(); });
urlInput.addEventListener("keydown", (e) => { if (e.key === "Enter" && !submitBtn.disabled) void submit(); });

useTabBtn.addEventListener("click", () => void fillTab());

async function fillTab() {
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (tab?.url && isHttpUrl(tab.url) && !hasEmbeddedCredentials(tab.url)) {
      urlInput.value = tab.url; clearResult(); updateSubmit();
    } else {
      showResult("error", "Current page is not a downloadable HTTP/HTTPS URL.");
    }
  } catch { showResult("error", "Could not read the current tab."); }
}

submitBtn.addEventListener("click", () => void submit());

async function submit() {
  const url = urlInput.value.trim();
  const err = validateUrl(url);
  if (err) { showResult("error", err); return; }
  clearResult();
  submitBtn.disabled = true;
  btnText.textContent = "Sending…";
  try {
    const resp = await chrome.runtime.sendMessage({ type: "add_download", url }) as ReturnType<typeof sendToNativeHost> extends Promise<infer R> ? R : never;
    if (isError(resp)) {
      showResult("error", userMessageForCode(resp.error.code));
    } else {
      showResult("success", "✓ Download added to MyDownloader");
      urlInput.value = "";
      updateSubmit();
    }
  } catch (e) {
    showResult("error", e instanceof NativeHostError ? e.message : "Could not reach MyDownloader.");
  } finally {
    btnText.textContent = "Download with MyDownloader";
    updateSubmit();
  }
}

// ── Tasks tab ─────────────────────────────────────────────────────────────

const STATUS_ICON: Record<string, string> = {
  downloading: "↓", completed: "✓", failed: "✗",
  queued: "…", preparing: "↓", paused: "‖", cancelled: "×",
};

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 ** 2) return `${(b/1024).toFixed(1)} KB`;
  if (b < 1024 ** 3) return `${(b/1024**2).toFixed(1)} MB`;
  return `${(b/1024**3).toFixed(2)} GB`;
}

function fmtSpeed(b: number | null): string {
  if (!b) return "";
  return `${fmtBytes(b)}/s`;
}

interface DL { id:string;filename:string;url:string;status:string;downloaded_bytes:number;total_bytes:number|null;percentage_basis_points:number|null;speed_bytes_per_second:number|null; }

function renderTasks(downloads: DL[]) {
  if (!downloads.length) { taskList.innerHTML = '<p class="empty-msg">No downloads yet</p>'; return; }
  taskList.innerHTML = downloads.map((d) => {
    const pct = d.percentage_basis_points != null ? Math.round(d.percentage_basis_points / 100) : null;
    const icon = STATUS_ICON[d.status] ?? "↓";
    const speed = fmtSpeed(d.speed_bytes_per_second);
    const meta = [fmtBytes(d.downloaded_bytes), pct != null ? `${pct}%` : "", speed].filter(Boolean).join(" · ");
    const bar = pct != null && d.status === "downloading"
      ? `<div class="task-progress-bar"><div class="task-progress-fill" style="width:${pct}%"></div></div>` : "";
    return `<div class="task-item">
      <div class="task-icon st-${d.status}">${icon}</div>
      <div class="task-info">
        <div class="task-name" title="${esc(d.filename)}">${esc(d.filename)}</div>
        <div class="task-meta"><span>${d.status}</span><span>${meta}</span></div>
        ${bar}
      </div>
    </div>`;
  }).join("");
}

async function loadTasks() {
  taskList.innerHTML = '<p class="empty-msg">Loading…</p>';
  try {
    const tasks = await chrome.runtime.sendMessage({ type: "get_tasks" }) as DL[];
    renderTasks(Array.isArray(tasks) ? tasks : []);
  } catch { taskList.innerHTML = '<p class="empty-msg">Could not load tasks</p>'; }
}

refreshBtn.addEventListener("click", () => {
  void chrome.runtime.sendMessage({ type: "refresh_tasks" }).then(() => loadTasks());
});

// ── Resources tab ─────────────────────────────────────────────────────────

interface Resource { url: string; type: string; label: string; }

let currentResources: Resource[] = [];
let currentFilter = "all";
const selectedUrls = new Set<string>();

document.querySelectorAll<HTMLButtonElement>(".filter-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".filter-btn").forEach((b) => b.classList.remove("filter-active"));
    btn.classList.add("filter-active");
    currentFilter = btn.dataset.filter ?? "all";
    renderResources();
  });
});

scanBtn.addEventListener("click", () => void scanPage());

async function scanPage() {
  scanBtn.textContent = "Scanning…";
  scanBtn.disabled = true;
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (!tab?.id) throw new Error("No active tab");
    const results = await chrome.tabs.sendMessage(tab.id, { type: "scan_resources" }) as Resource[];
    currentResources = Array.isArray(results) ? results : [];
    selectedUrls.clear();
    scanEmpty.classList.add("hidden");
    resourceList.classList.remove("hidden");
    renderResources();
  } catch {
    scanBtn.textContent = "🔍 Scan this page";
    scanBtn.disabled = false;
    scanEmpty.innerHTML = '<p style="font-size:12px;color:var(--text-dim);text-align:center">Could not scan this page.<br>Reload the page and try again.</p>';
  }
}

function renderResources() {
  const filtered = currentFilter === "all"
    ? currentResources
    : currentResources.filter((r) => r.type === currentFilter);

  if (!filtered.length) {
    resourceList.innerHTML = `<p class="empty-msg">No ${currentFilter === "all" ? "" : currentFilter + " "}resources found</p>`;
    updateSelectionUI();
    return;
  }

  resourceList.innerHTML = filtered.map((r) => {
    const checked = selectedUrls.has(r.url) ? "checked" : "";
    const sel = selectedUrls.has(r.url) ? " selected" : "";
    return `<label class="resource-item${sel}" data-url="${esc(r.url)}">
      <input type="checkbox" ${checked} data-url="${esc(r.url)}" />
      <span class="resource-type-badge badge-${r.type}">${r.type}</span>
      <span class="resource-name" title="${esc(r.url)}">${esc(r.label)}</span>
    </label>`;
  }).join("");

  resourceList.querySelectorAll<HTMLInputElement>("input[type=checkbox]").forEach((cb) => {
    cb.addEventListener("change", () => {
      const url = cb.dataset.url!;
      cb.checked ? selectedUrls.add(url) : selectedUrls.delete(url);
      const row = cb.closest(".resource-item");
      row?.classList.toggle("selected", cb.checked);
      updateSelectionUI();
    });
  });

  updateSelectionUI();
}

function updateSelectionUI() {
  const n = selectedUrls.size;
  selectedCount.textContent = `${n} selected`;
  dlSelectedBtn.disabled = n === 0;
  resActions.classList.toggle("hidden", currentResources.length === 0);
}

dlSelectedBtn.addEventListener("click", () => void downloadSelected());

async function downloadSelected() {
  if (!selectedUrls.size) return;
  dlSelectedBtn.disabled = true;
  dlSelectedBtn.textContent = "Sending…";
  try {
    const result = await chrome.runtime.sendMessage({
      type: "add_batch",
      urls: [...selectedUrls],
    }) as { added: number; failed: number };
    selectedUrls.clear();
    renderResources();
    const msg = result.failed === 0
      ? `✓ ${result.added} download${result.added > 1 ? "s" : ""} added`
      : `${result.added} added, ${result.failed} failed`;
    // Show brief feedback in the Add tab result
    showResult(result.failed === 0 ? "success" : "error", msg);
    // Switch to tasks tab
    document.querySelector<HTMLButtonElement>('[data-tab="tasks"]')?.click();
  } finally {
    dlSelectedBtn.textContent = "↓ Download selected";
    updateSelectionUI();
  }
}

// ── Settings ──────────────────────────────────────────────────────────────

settingsBtn.addEventListener("click", () => { void loadSettings(); settingsPanel.classList.remove("hidden"); });
settingsClose.addEventListener("click", () => settingsPanel.classList.add("hidden"));

async function loadSettings() {
  const s = await chrome.runtime.sendMessage({ type: "get_settings" }) as ExtensionSettings;
  setAutoIntercept.checked = s.autoIntercept;
  setMinSize.value = String(s.minSizeMb);
  setNotif.checked = s.showNotifications;
}

settingsSave.addEventListener("click", () => void saveSettings());

async function saveSettings() {
  const settings: Partial<ExtensionSettings> = {
    autoIntercept: setAutoIntercept.checked,
    minSizeMb: Number(setMinSize.value) || 0,
    showNotifications: setNotif.checked,
  };
  await chrome.runtime.sendMessage({ type: "save_settings", settings });
  settingsPanel.classList.add("hidden");
}

// ── Helpers ───────────────────────────────────────────────────────────────

function esc(s: string): string {
  return s.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");
}

// ── Boot ──────────────────────────────────────────────────────────────────

async function init() {
  void fillTab();
  try {
    const resp = await sendToNativeHost({ version: PROTOCOL_VERSION, type: "ping" });
    if (resp.ok && "pong" in resp && resp.pong) {
      const ver = "app_version" in resp ? ` · v${resp.app_version}` : "";
      setStatus("connected", `Connected${ver}`);
    } else {
      setStatus("disconnected", "Not running");
    }
  } catch { setStatus("disconnected", "Not running"); }
}

void init();
