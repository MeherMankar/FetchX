import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-500.css";
import "./popup.css";
import { isError, userMessageForCode } from "../types/protocol";
import type { BrowserResponse, RpcRequest, RpcTarget } from "../types/protocol";
import {
  NativeHostError,
  callTargetRpc,
  deleteTargetCredential,
  hasEmbeddedCredentials,
  isHttpUrl,
  storeTargetCredential,
} from "../shared/native";
import { browserApi, sendRuntimeMessage } from "../shared/browser-api";
import type { BrowserTarget, ExtensionSettings } from "../background/service-worker";
import { formatIdForDownload, isVideoOnlyFormat } from "./media-format";
import type { MediaFormat } from "./media-format";
import type { InterceptRule } from "../types/rules";

// ──DOM refs ──────────────────────────────────────────────────────────────

const statusBadge    = q<HTMLElement>("#status-badge");
const statusText     = q<HTMLElement>("#status-text");
const urlInput       = q<HTMLInputElement>("#url-input");
const useTabBtn      = q<HTMLButtonElement>("#use-tab-btn");
const submitBtn      = q<HTMLButtonElement>("#submit-btn");
const fetchFormatsBtn = q<HTMLButtonElement>("#fetch-formats-btn");
const detectedMedia  = q<HTMLElement>("#media-detected");
const detectedMediaText = q<HTMLElement>("#media-detected-text");
const detectedMediaBtn = q<HTMLButtonElement>("#detected-media-btn");
const scanPageBtn    = q<HTMLButtonElement>("#scan-page-btn");
const qualityPanel   = q<HTMLElement>("#quality-panel");
const qualityTitle   = q<HTMLElement>("#quality-title");
const qualityList    = q<HTMLElement>("#quality-list");
const downloadAllFormatsBtn = q<HTMLButtonElement>("#download-all-formats-btn");
const btnText        = q<HTMLElement>("#btn-text");
const resultBanner   = q<HTMLElement>("#result-banner");
const taskList       = q<HTMLElement>("#task-list");
const refreshBtn     = q<HTMLButtonElement>("#refresh-tasks-btn");
const scanBtn        = q<HTMLButtonElement>("#scan-btn");
const scanEmpty      = q<HTMLElement>("#scan-empty");
const resourceList   = q<HTMLElement>("#resource-list");
const resActions     = q<HTMLElement>("#resource-actions");
const selectedCount  = q<HTMLElement>("#selected-count");
const dlSelectedBtn  = q<HTMLButtonElement>("#download-selected-btn");
const settingsBtn    = q<HTMLButtonElement>("#settings-btn");
const settingsPanel  = q<HTMLElement>("#settings-panel");
const settingsClose  = q<HTMLButtonElement>("#settings-close-btn");
const settingsSave   = q<HTMLButtonElement>("#settings-save-btn");
const setAutoIntercept = q<HTMLInputElement>("#set-auto-intercept");
const setMinSize     = q<HTMLInputElement>("#set-min-size");
const setNotif       = q<HTMLInputElement>("#set-notifications");
const targetSelect   = q<HTMLSelectElement>("#target-select");
const targetList     = q<HTMLElement>("#target-list");
const targetName     = q<HTMLInputElement>("#target-name");
const targetEndpoint = q<HTMLInputElement>("#target-endpoint");
const targetToken    = q<HTMLInputElement>("#target-token");
const targetAddBtn   = q<HTMLButtonElement>("#target-add-btn");
const targetResult   = q<HTMLElement>("#target-result");

function q<T extends Element>(sel: string): T {
  return document.querySelector<T>(sel)!;
}

// ──Tabs ──────────────────────────────────────────────────────────────────

document.querySelectorAll<HTMLButtonElement>(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => {
      b.classList.remove("tab-active");
      b.setAttribute("aria-selected", "false");
    });
    document.querySelectorAll<HTMLElement>(".tab-panel").forEach((p) => p.classList.add("hidden"));
    btn.classList.add("tab-active");
    btn.setAttribute("aria-selected", "true");
    const panel = document.getElementById(`tab-${btn.dataset.tab}`);
    panel?.classList.remove("hidden");
    if (btn.dataset.tab === "tasks") void loadTasks();
    if (btn.dataset.tab === "drm") void loadDrmKeys();
  });
});

// ──Status ────────────────────────────────────────────────────────────────

function setStatus(state: "checking" | "connected" | "disconnected", label?: string) {
  statusBadge.className = `status-chip status-${state}`;
  statusText.textContent = label ?? { checking: "Checking…", connected: "Connected", disconnected: "Not running" }[state];
}

// ──Result banner ─────────────────────────────────────────────────────────

function showResult(kind: "success" | "error", msg: string) {
  resultBanner.textContent = msg;
  resultBanner.className = `banner ${kind}`;
}
function clearResult() {
  resultBanner.className = "banner hidden";
  resultBanner.textContent = "";
}

// ──URL / Add tab ─────────────────────────────────────────────────────────

function validateUrl(url: string): string | null {
  if (!url.trim()) return "Please enter a URL.";
  if (!isHttpUrl(url)) return "Only HTTP and HTTPS URLs are supported.";
  if (hasEmbeddedCredentials(url)) return "URLs with credentials are not accepted.";
  return null;
}

function updateSubmit() {
  const invalid = validateUrl(urlInput.value.trim()) !== null;
  submitBtn.disabled = invalid;
  fetchFormatsBtn.disabled = invalid;
}

interface MediaInfo {
  title: string;
  formats: MediaFormat[];
}

let availableFormats: MediaFormat[] = [];

urlInput.addEventListener("input", () => {
  clearResult();
  availableFormats = [];
  qualityPanel.classList.add("hidden");
  updateSubmit();
});
urlInput.addEventListener("keydown", (e) => { if (e.key === "Enter" && !submitBtn.disabled) void submit(); });

useTabBtn.addEventListener("click", () => void fillTab());
fetchFormatsBtn.addEventListener("click", () => void fetchVideoFormats());
detectedMediaBtn.addEventListener("click", () => void chooseDetectedMediaQuality());
scanPageBtn.addEventListener("click", () => {
  document.querySelector<HTMLButtonElement>('[data-tab="resources"]')?.click();
  void scanPage();
});
downloadAllFormatsBtn.addEventListener("click", () => void downloadFormats(availableFormats));

async function fillTab(): Promise<boolean> {
  try {
    const [tab] = await browserApi.tabs.query({ active: true, currentWindow: true });
    if (tab?.url && isHttpUrl(tab.url) && !hasEmbeddedCredentials(tab.url)) {
      urlInput.value = tab.url; clearResult(); updateSubmit();
      return true;
    } else {
      showResult("error", "Current page is not a downloadable HTTP/HTTPS URL.");
    }
  } catch { showResult("error", "Could not read the current tab."); }
  return false;
}

async function refreshDetectedMedia() {
  try {
    const [tab] = await browserApi.tabs.query({ active: true, currentWindow: true });
    if (!tab?.id || !tab.url || !isHttpUrl(tab.url) || hasEmbeddedCredentials(tab.url)) return;
    const streams = await sendRuntimeMessage<NetworkStream[]>({
      type: "get_media_for_tab",
      tabId: tab.id,
    });
    const count = Array.isArray(streams) ? streams.length : 0;
    if (count === 0) return;
    detectedMediaText.textContent =
      `FetchX detected ${count} media stream${count === 1 ? "" : "s"} on this page.`;
    detectedMedia.classList.remove("hidden");
  } catch (error) {
    console.error("Could not check for detected media", error);
  }
}

async function chooseDetectedMediaQuality() {
  if (!await fillTab()) return;
  await fetchVideoFormats();
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
    const resp = await sendRuntimeMessage<BrowserResponse>({ type: "add_download", url });
    if (isError(resp)) {
      showResult("error", userMessageForCode(resp.error.code));
    } else {
      showResult("success", "Download added to FetchX");
      urlInput.value = "";
      updateSubmit();
    }
  } catch (e) {
    showResult("error", e instanceof NativeHostError ? e.message : "Could not reach FetchX.");
  } finally {
    btnText.textContent = "Add to FetchX";
    updateSubmit();
  }
}

async function fetchVideoFormats() {
  const url = urlInput.value.trim();
  const err = validateUrl(url);
  if (err) { showResult("error", err); return; }
  fetchFormatsBtn.disabled = true;
  fetchFormatsBtn.textContent = "Finding qualities…";
  clearResult();
  try {
    const target = await selectedRpcTarget();
    if (target.kind !== "local") {
      throw new Error("Video quality selection is currently available for This PC only.");
    }
    const response = await callTargetRpc(target, {
      jsonrpc: "2.0",
      id: crypto.randomUUID(),
      method: "media.extractFormats",
      params: { url },
    });
    const rpcResponse = getRpcResponse(response);
    if ("error" in rpcResponse) {
      throw new Error(rpcErrorMessage(rpcResponse) ?? "Could not fetch video qualities.");
    }
    const info = parseMediaInfo(rpcResponse.result);
    availableFormats = info.formats;
    qualityTitle.textContent = info.title || "Available formats";
    renderVideoFormats();
    qualityPanel.classList.remove("hidden");
    if (availableFormats.length === 0) {
      showResult("error", "No downloadable video qualities were found.");
    }
  } catch (error) {
    showResult("error", error instanceof Error ? error.message : "Could not fetch video qualities.");
  } finally {
    fetchFormatsBtn.textContent = "Fetch video qualities";
    updateSubmit();
  }
}

function renderVideoFormats() {
  qualityList.replaceChildren();
  availableFormats.forEach((format, index) => {
    const row = document.createElement("div");
    row.className = "quality-item";
    const label = document.createElement("span");
    label.className = "quality-label";
    label.textContent = `${index + 1}. ${format.label || format.ext}${isVideoOnlyFormat(format) ? " + audio" : ""}`;
    const download = document.createElement("button");
    download.type = "button";
    download.className = "btn-text";
    download.textContent = "Download";
    download.addEventListener("click", () => void downloadFormats([format]));
    row.append(label, download);
    qualityList.append(row);
  });
  downloadAllFormatsBtn.disabled = availableFormats.length === 0;
}

async function downloadFormats(formats: MediaFormat[]) {
  if (!formats.length) return;
  const url = urlInput.value.trim();
  const err = validateUrl(url);
  if (err) { showResult("error", err); return; }
  downloadAllFormatsBtn.disabled = true;
  for (const button of Array.from(qualityList.querySelectorAll<HTMLButtonElement>("button"))) {
    button.disabled = true;
  }
  let added = 0;
  let failed = 0;
  try {
    const target = await selectedRpcTarget();
    if (target.kind !== "local") {
      throw new Error("Video quality selection is currently available for This PC only.");
    }
    for (const format of formats) {
      const response = await callTargetRpc(target, {
        jsonrpc: "2.0",
        id: crypto.randomUUID(),
        method: "media.addFormat",
        params: { url, format_id: formatIdForDownload(format), extension: format.ext },
      });
      const rpcResponse = getRpcResponse(response);
      if ("error" in rpcResponse) {
        failed++;
      } else {
        added++;
      }
    }
    const message = failed === 0
      ? `Added ${added} format${added === 1 ? "" : "s"} to FetchX`
      : `${added} added, ${failed} failed. Check yt-dlp in FetchX settings.`;
    showResult(failed === 0 ? "success" : "error", message);
  } catch (error) {
    showResult("error", error instanceof Error ? error.message : "Could not add the selected formats.");
  } finally {
    downloadAllFormatsBtn.disabled = availableFormats.length === 0;
    for (const button of Array.from(qualityList.querySelectorAll<HTMLButtonElement>("button"))) {
      button.disabled = false;
    }
  }
}

function rpcErrorMessage(response: Record<string, unknown>): string | undefined {
  const error = response.error;
  if (typeof error !== "object" || error === null) return undefined;
  const message = (error as Record<string, unknown>).message;
  return typeof message === "string" ? message : undefined;
}

function getRpcResponse(response: BrowserResponse): Record<string, unknown> {
  if (isError(response)) {
    throw new Error(userMessageForCode(response.error.code));
  }
  if (!("rpc_response" in response) || typeof response.rpc_response !== "object"
    || response.rpc_response === null) {
    throw new Error("FetchX returned an invalid media response.");
  }
  const rpcResponse = response.rpc_response;
  if (rpcResponse.jsonrpc !== "2.0") {
    throw new Error("FetchX returned an invalid media response.");
  }
  return rpcResponse;
}

function parseMediaInfo(value: unknown): MediaInfo {
  if (typeof value !== "object" || value === null) {
    throw new Error("FetchX returned invalid video qualities.");
  }
  const candidate = value as Record<string, unknown>;
  if (typeof candidate.title !== "string" || !Array.isArray(candidate.formats)) {
    throw new Error("FetchX returned invalid video qualities.");
  }
  const formats = candidate.formats.filter((format): format is MediaFormat => {
    if (typeof format !== "object" || format === null) return false;
    const record = format as Record<string, unknown>;
    const hasValidCodec = (value: unknown) =>
      value === undefined || value === null || typeof value === "string";
    return typeof record.format_id === "string"
      && typeof record.label === "string"
      && typeof record.ext === "string"
      && hasValidCodec(record.video_codec)
      && hasValidCodec(record.audio_codec);
  });
  return { title: candidate.title, formats };
}

// ──Tasks tab ─────────────────────────────────────────────────────────────

const STATUS_CLASS: Record<string, string> = {
  downloading: "downloading",
  metadata_acquiring: "downloading",
  recovering: "downloading",
  seeding: "downloading",
  completed: "completed",
  failed: "failed",
  queued: "queued",
  preparing: "downloading",
  paused: "paused",
  cancelled: "cancelled",
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

interface DL {
  id: string;
  filename: string;
  url: string;
  status: string;
  downloaded_bytes: number;
  total_bytes: number | null;
  percentage_basis_points: number | null;
  speed_bytes_per_second: number | null;
}

function renderTasks(downloads: DL[]) {
  if (!downloads.length) {
    taskList.innerHTML = `<div class="empty-state"><p>No downloads yet</p></div>`;
    return;
  }
  taskList.innerHTML = downloads.map((d) => {
    const pct = d.percentage_basis_points != null
      ? Math.min(100, Math.max(0, Math.round(d.percentage_basis_points / 100)))
      : null;
    const meta = d.status.replaceAll("_", " ");
    const statusClass = STATUS_CLASS[d.status] ?? "queued";
    const speed = fmtSpeed(d.speed_bytes_per_second);
    const sizeInfo = [fmtBytes(d.downloaded_bytes), pct != null ? `${pct}%` : ""].filter(Boolean).join(" · ");
    const progress = pct != null && d.status === "downloading"
      ? `<progress class="task-progress" value="${pct}" max="100" aria-label="Progress for ${esc(d.filename)}"></progress>` : "";
    return `<div class="task-item" data-status="${esc(statusClass)}">
      <div class="task-header">
        <div class="task-name" title="${esc(d.filename)}">${esc(d.filename)}</div>
        <span class="task-status task-status-${esc(statusClass)}">${esc(meta)}</span>
      </div>
      <div class="task-body">
        <div class="task-meta">
          <span>${esc(sizeInfo)}</span>
          <span>${esc(speed)}</span>
        </div>
        ${progress}
      </div>
    </div>`;
  }).join("");
}

async function loadTasks() {
  taskList.innerHTML = `<div class="empty-state"><p>Loading…</p></div>`;
  try {
    const tasks = await sendRuntimeMessage<DL[]>({ type: "get_tasks" });
    renderTasks(Array.isArray(tasks) ? tasks : []);
  } catch {
    taskList.innerHTML = `<div class="empty-state"><p>Could not load tasks</p></div>`;
  }
}

refreshBtn.addEventListener("click", () => {
  void sendRuntimeMessage<unknown>({ type: "refresh_tasks" }).then(() => loadTasks());
});

// ──Resources tab ─────────────────────────────────────────────────────────

interface Resource { url: string; type: string; label: string; }

let currentResources: Resource[] = [];
let currentFilter = "all";
const selectedUrls = new Set<string>();

document.querySelectorAll<HTMLButtonElement>(".filter-pill").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".filter-pill").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    currentFilter = btn.dataset.filter ?? "all";
    renderResources();
  });
});

scanBtn.addEventListener("click", () => void scanPage());

async function scanPage() {
  scanBtn.querySelector("span:last-child")!.textContent = "Scanning…";
  scanBtn.disabled = true;
  try {
    const [tab] = await browserApi.tabs.query({ active: true, currentWindow: true });
    if (!tab?.id) throw new Error("No active tab");

    // Run DOM scan (content script) + network media query (background) in parallel
    const [domResults, networkMedia] = await Promise.all([
      browserApi.tabs.sendMessage(tab.id, { type: "scan_resources" }) as Promise<Resource[]>,
      sendRuntimeMessage<NetworkStream[]>({ type: "get_media_for_tab", tabId: tab.id }),
    ]);

    const domResources: Resource[] = Array.isArray(domResults) ? domResults : [];
    const networkResources: Resource[] = (Array.isArray(networkMedia) ? networkMedia : []).map(streamToResource);
    // Merge, deduplicating by URL (DOM-scan results take priority)
    const seen = new Set(domResources.map((r) => r.url));
    const merged = [...domResources, ...networkResources.filter((r) => !seen.has(r.url))];

    currentResources = merged;
    selectedUrls.clear();
    scanEmpty.classList.add("hidden");
    resourceList.classList.remove("hidden");
    renderResources();
  } catch {
    scanBtn.querySelector("span:last-child")!.textContent = "Scan this page";
    scanBtn.disabled = false;
    scanEmpty.innerHTML = `<p>Could not scan this page.<br>Reload the page and try again.</p>`;
  }
}

interface NetworkStream {
  kind: string;
  url: string;
  suggestedFilename: string;
  mimeType: string | null;
  contentLength: number | null;
}

function streamToResource(s: NetworkStream): Resource {
  return {
    url: s.url,
    type: s.kind === "audio" ? "audio" : "video",
    label: s.suggestedFilename || s.url.split("/").pop() || s.url.slice(0, 60),
  };
}

function renderResources() {
  const filtered = currentFilter === "all"
    ? currentResources
    : currentResources.filter((r) => r.type === currentFilter);

  if (!filtered.length) {
    resourceList.innerHTML = `<div class="empty-state"><p>No ${currentFilter === "all" ? "" : currentFilter + " "}resources found</p></div>`;
    updateSelectionUI();
    return;
  }

  resourceList.innerHTML = filtered.map((r) => {
    const checked = selectedUrls.has(r.url) ? "checked" : "";
    const sel = selectedUrls.has(r.url) ? " selected" : "";
    const type = ["video", "audio", "image", "file"].includes(r.type) ? r.type : "file";
    return `<label class="resource-item${sel}" data-url="${esc(r.url)}">
      <input type="checkbox" ${checked} data-url="${esc(r.url)}" />
      <span class="resource-type resource-type-${type}">${type}</span>
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
  dlSelectedBtn.querySelector("span:last-child")!.textContent = "Sending…";
  try {
    const result = await sendRuntimeMessage<{ added: number; failed: number }>({
      type: "add_batch",
      urls: [...selectedUrls],
    });
    selectedUrls.clear();
    renderResources();
    const msg = result.failed === 0
      ? `${result.added} download${result.added === 1 ? "" : "s"} added`
      : `${result.added} added, ${result.failed} failed`;
    showResult(result.failed === 0 ? "success" : "error", msg);
    document.querySelector<HTMLButtonElement>('[data-tab="tasks"]')?.click();
  } finally {
    dlSelectedBtn.querySelector("span:last-child")!.textContent = "Download selected";
    updateSelectionUI();
  }
}

// ──Settings ──────────────────────────────────────────────────────────────

settingsBtn.addEventListener("click", () => { void loadSettings(); settingsPanel.classList.remove("hidden"); });
settingsClose.addEventListener("click", () => settingsPanel.classList.add("hidden"));
targetSelect.addEventListener("change", () => void selectTarget(targetSelect.value));
targetAddBtn.addEventListener("click", () => void addRemoteTarget());

async function loadSettings() {
  const s = await sendRuntimeMessage<ExtensionSettings>({ type: "get_settings" });
  setAutoIntercept.checked = s.autoIntercept;
  setMinSize.value = String(s.minSizeMb);
  setNotif.checked = s.showNotifications;
  await loadTargets();
  await loadRules();
}

settingsSave.addEventListener("click", () => void saveSettings());

async function saveSettings() {
  const settings: Partial<ExtensionSettings> = {
    autoIntercept: setAutoIntercept.checked,
    minSizeMb: Number(setMinSize.value) || 0,
    showNotifications: setNotif.checked,
  };
  await sendRuntimeMessage<unknown>({ type: "save_settings", settings });
  settingsPanel.classList.add("hidden");
}

const LOCAL_TARGET: BrowserTarget = {
  id: "local-windows",
  name: "This PC",
  kind: "local",
};

async function loadTargets() {
  const stored = await browserApi.storage.local.get(["browserTargets", "selectedTargetId"]);
  const remoteTargets = Array.isArray(stored.browserTargets)
    ? stored.browserTargets.filter(isBrowserTarget)
    : [];
  const targets = [LOCAL_TARGET, ...remoteTargets.filter((target) => target.kind === "remote")];
  const selectedId = typeof stored.selectedTargetId === "string"
    && targets.some((target) => target.id === stored.selectedTargetId)
    ? stored.selectedTargetId
    : LOCAL_TARGET.id;

  targetSelect.replaceChildren();
  for (const target of targets) {
    const option = document.createElement("option");
    option.value = target.id;
    option.textContent = target.name;
    targetSelect.append(option);
  }
  targetSelect.value = selectedId;

  targetList.replaceChildren();
  for (const target of remoteTargets) {
    const row = document.createElement("div");
    row.className = "target-card";
    const label = document.createElement("span");
    label.className = "target-name-ep";
    label.textContent = `${target.name} · ${target.endpoint}`;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "btn-remove";
    remove.textContent = "Remove";
    remove.addEventListener("click", () => void removeTarget(target));
    row.append(label, remove);
    targetList.append(row);
  }
}

async function selectTarget(targetId: string) {
  await browserApi.storage.local.set({ selectedTargetId: targetId });
  setStatus("checking", "Checking target…");
  await checkConnection();
  void sendRuntimeMessage<unknown>({ type: "target_changed" });
}

async function addRemoteTarget() {
  const name = targetName.value.trim();
  const endpoint = validateRemoteEndpoint(targetEndpoint.value.trim());
  const token = targetToken.value;
  if (!name || !endpoint || !token) {
    showTargetResult("error", "Enter a name, HTTPS server origin, and bearer token.");
    return;
  }
  targetAddBtn.disabled = true;
  showTargetResult("success", "Saving credential securely…");
  const credentialId = crypto.randomUUID();
  let credentialStored = false;
  try {
    const stored = await storeTargetCredential(credentialId, endpoint, token);
    targetToken.value = "";
    if (!stored.ok) throw new Error(userMessageForCode(stored.error.code));
    credentialStored = true;
    const target: BrowserTarget = {
      id: credentialId,
      name,
      kind: "remote",
      endpoint,
      credential_id: credentialId,
    };
    const current = await browserApi.storage.local.get("browserTargets");
    const targets = Array.isArray(current.browserTargets)
      ? current.browserTargets.filter(isBrowserTarget).filter((entry) => entry.kind === "remote")
      : [];
    await browserApi.storage.local.set({
      browserTargets: [...targets, target],
      selectedTargetId: target.id,
    });
    targetName.value = "";
    targetEndpoint.value = "";
    await loadTargets();
    showTargetResult("success", "Target added. The token is stored by Windows, not in extension storage.");
    await checkConnection();
  } catch (error) {
    if (credentialStored) {
      await deleteTargetCredential(credentialId, endpoint);
    }
    showTargetResult(
      "error",
      error instanceof Error ? error.message : "Could not save this target.",
    );
  } finally {
    targetToken.value = "";
    targetAddBtn.disabled = false;
  }
}

async function removeTarget(target: BrowserTarget) {
  if (target.kind !== "remote") return;
  try {
    const deleted = await deleteTargetCredential(target.credential_id, target.endpoint);
    if (!deleted.ok) {
      showTargetResult("error", userMessageForCode(deleted.error.code));
      return;
    }
    const current = await browserApi.storage.local.get(["browserTargets", "selectedTargetId"]);
    const targets = Array.isArray(current.browserTargets)
      ? current.browserTargets.filter(isBrowserTarget).filter((entry) => entry.id !== target.id)
      : [];
    await browserApi.storage.local.set({
      browserTargets: targets,
      selectedTargetId: current.selectedTargetId === target.id
        ? LOCAL_TARGET.id
        : current.selectedTargetId,
    });
    await loadTargets();
    await checkConnection();
  } catch (error) {
    showTargetResult(
      "error",
      error instanceof Error ? error.message : "Could not remove this target.",
    );
  }
}

async function selectedRpcTarget(): Promise<RpcTarget> {
  const stored = await browserApi.storage.local.get(["browserTargets", "selectedTargetId"]);
  const targets = Array.isArray(stored.browserTargets)
    ? stored.browserTargets.filter(isBrowserTarget)
    : [];
  const target = targets.find((entry) => entry.id === stored.selectedTargetId) ?? LOCAL_TARGET;
  if (target.kind === "local") return { kind: "local" };
  return {
    kind: "remote",
    endpoint: target.endpoint,
    credential_id: target.credential_id,
  };
}

async function checkConnection() {
  try {
    const request: RpcRequest = {
      jsonrpc: "2.0",
      id: crypto.randomUUID(),
      method: "system.discover",
    };
    const response = await callTargetRpc(await selectedRpcTarget(), request);
    if (!response.ok || !("rpc_response" in response)) {
      if (response.ok) {
        setStatus("disconnected", "Target returned an invalid RPC response.");
        return;
      }
      setStatus("disconnected", userMessageForCode(response.error.code));
      return;
    }
    const rpcResponse = response.rpc_response;
    if ("error" in rpcResponse) {
      setStatus("disconnected", "Target rejected the RPC request.");
      return;
    }
    const result = rpcResponse.result as { protocol_version?: unknown } | undefined;
    if (result?.protocol_version !== "fetchx.rpc.v1") {
      setStatus("disconnected", "Incompatible FetchX RPC server");
      return;
    }
    setStatus("connected", "Connected · fetchx.rpc.v1");
  } catch (error) {
    setStatus(
      "disconnected",
      error instanceof NativeHostError ? error.message : "Could not reach the selected target.",
    );
  }
}

function validateRemoteEndpoint(value: string): string | null {
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || url.username || url.password
      || (url.pathname !== "/" && url.pathname !== "")
      || url.search || url.hash) return null;
    return url.origin;
  } catch {
    return null;
  }
}

function isBrowserTarget(
  value: unknown,
): value is Extract<BrowserTarget, { kind: "remote" }> {
  if (typeof value !== "object" || value === null) return false;
  const target = value as Record<string, unknown>;
  return typeof target.id === "string"
    && typeof target.name === "string"
    && target.kind === "remote"
    && typeof target.endpoint === "string"
    && typeof target.credential_id === "string";
}

function showTargetResult(kind: "success" | "error", message: string) {
  targetResult.textContent = message;
  targetResult.className = `banner ${kind}`;
}

// ──Helpers ───────────────────────────────────────────────────────────────

function esc(s: string): string {
  return s.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");
}

// ──Rules ─────────────────────────────────────────────────────────────────

let currentRules: InterceptRule[] = [];

const rulesList = q<HTMLElement>("#rules-list");
const ruleAddForm = q<HTMLElement>("#rule-add-form");
const addRuleBtn = q<HTMLButtonElement>("#add-rule-btn");
const ruleFormCancel = q<HTMLButtonElement>("#rule-form-cancel");
const ruleFormSave = q<HTMLButtonElement>("#rule-form-save");
const ruleNameInput = q<HTMLInputElement>("#rule-name");
const ruleMatchType = q<HTMLSelectElement>("#rule-match-type");
const ruleMatchValue = q<HTMLInputElement>("#rule-match-value");
const ruleActionSelect = q<HTMLSelectElement>("#rule-action");

function matchSummary(rule: InterceptRule): string {
  switch (rule.match.kind) {
    case "domain": return `Domain: ${rule.match.domain}`;
    case "url_pattern": return `Pattern: ${rule.match.pattern}`;
    case "file_extension": return `Ext: .${rule.match.ext}`;
  }
}

function renderRules() {
  rulesList.replaceChildren();
  if (!currentRules.length) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.innerHTML = `<p>No rules yet. Add one to override auto-intercept behaviour.</p>`;
    rulesList.append(empty);
    return;
  }
  for (const rule of currentRules) {
    const item = document.createElement("div");
    item.className = "rule-item";

    const info = document.createElement("div");
    info.className = "rule-item-info";

    const name = document.createElement("span");
    name.className = "rule-item-name";
    name.textContent = rule.name;

    const summary = document.createElement("span");
    summary.className = "rule-item-summary";
    summary.textContent = `${matchSummary(rule)} · ${rule.action === "always_intercept" ? "Always intercept" : "Never intercept"}`;

    info.append(name, summary);

    const actions = document.createElement("div");
    actions.className = "rule-item-actions";

    const toggle = document.createElement("input");
    toggle.type = "checkbox";
    toggle.className = "toggle";
    toggle.checked = rule.enabled;
    toggle.setAttribute("aria-label", `Enable rule ${rule.name}`);
    toggle.addEventListener("change", () => void toggleRule(rule.id, toggle.checked));

    const del = document.createElement("button");
    del.type = "button";
    del.className = "btn-text rule-delete-btn";
    del.textContent = "Delete";
    del.setAttribute("aria-label", `Delete rule ${rule.name}`);
    del.addEventListener("click", () => void deleteRule(rule.id));

    actions.append(toggle, del);
    item.append(info, actions);
    rulesList.append(item);
  }
}

async function loadRules() {
  currentRules = await sendRuntimeMessage<InterceptRule[]>({ type: "get_intercept_rules" });
  if (!Array.isArray(currentRules)) currentRules = [];
  renderRules();
}

async function saveRules() {
  await sendRuntimeMessage<unknown>({ type: "set_intercept_rules", rules: currentRules });
}

async function toggleRule(id: string, enabled: boolean) {
  currentRules = currentRules.map((r) => r.id === id ? { ...r, enabled } : r);
  await saveRules();
}

async function deleteRule(id: string) {
  currentRules = currentRules.filter((r) => r.id !== id);
  await saveRules();
  renderRules();
}

addRuleBtn.addEventListener("click", () => {
  ruleAddForm.classList.remove("hidden");
  addRuleBtn.classList.add("hidden");
  ruleNameInput.value = "";
  ruleMatchType.value = "domain";
  ruleMatchValue.value = "";
  ruleActionSelect.value = "never_intercept";
  ruleNameInput.focus();
});

ruleFormCancel.addEventListener("click", () => {
  ruleAddForm.classList.add("hidden");
  addRuleBtn.classList.remove("hidden");
});

ruleFormSave.addEventListener("click", () => void saveNewRule());

async function saveNewRule() {
  const name = ruleNameInput.value.trim();
  const matchType = ruleMatchType.value as "domain" | "url_pattern" | "file_extension";
  const matchValue = ruleMatchValue.value.trim();
  const action = ruleActionSelect.value as "always_intercept" | "never_intercept";

  if (!name || !matchValue) return;

  let match: InterceptRule["match"];
  if (matchType === "domain") {
    match = { kind: "domain", domain: matchValue };
  } else if (matchType === "url_pattern") {
    match = { kind: "url_pattern", pattern: matchValue };
  } else {
    match = { kind: "file_extension", ext: matchValue.replace(/^\.+/, "") };
  }

  const rule: InterceptRule = {
    id: crypto.randomUUID(),
    enabled: true,
    name,
    match,
    action,
  };
  currentRules = [...currentRules, rule];
  await saveRules();
  renderRules();
  ruleAddForm.classList.add("hidden");
  addRuleBtn.classList.remove("hidden");
}

// ──Boot ──────────────────────────────────────────────────────────────────

async function init() {
  void fillTab();
  void refreshDetectedMedia();
  try {
    await loadTargets();
    await checkConnection();
  } catch {
    setStatus("disconnected", "Could not load FetchX target settings.");
  }
  initDrmTab();
}

// ──DRM Keys tab ──────────────────────────────────────────────────────────

interface WvpKeyEntry {
  url?: string;
  type?: string;
  keys?: Array<string | { kid: string; k: string }>;
  timestamp?: number;
}

const drmStatusBadge  = q<HTMLElement>("#drm-status-badge");
const drmStatusText   = q<HTMLElement>("#drm-status-text");
const drmKeyList      = q<HTMLElement>("#drm-key-list");
const drmHistoryBtn   = q<HTMLButtonElement>("#drm-history-btn");
const drmClearBtn     = q<HTMLButtonElement>("#drm-clear-btn");

function setDrmStatus(state: "checking" | "connected" | "disconnected", label: string) {
  drmStatusBadge.className = `status-chip status-${state}`;
  drmStatusBadge.style.cssText = "font-size:11px;padding:3px 7px;";
  drmStatusText.textContent = label;
}

function renderDrmKeys(entries: WvpKeyEntry[]) {
  if (!entries.length) {
    drmKeyList.innerHTML = `<div class="empty-state"><p>No keys captured yet</p></div>`;
    setDrmStatus("disconnected", "No keys");
    return;
  }
  setDrmStatus("connected", `${entries.length} entr${entries.length === 1 ? "y" : "ies"}`);
  drmKeyList.innerHTML = entries.slice(0, 20).map((entry) => {
    const keys = (entry.keys ?? []).map((k) =>
      typeof k === "string" ? k : `${k.kid}:${k.k}`,
    );
    const keyRows = keys.slice(0, 4).map((k) =>
      `<div class="drm-key-pair">${esc(k)}</div>`,
    ).join("");
    const label = drmTypeLabel(entry);
    return `<div class="drm-key-item">
      ${label ? `<span class="drm-key-type">${esc(label)}</span>` : ""}
      <div class="drm-key-url" title="${esc(entry.url ?? "")}">${esc(entry.url ?? "(unknown page)")}</div>
      <div class="drm-key-pairs">${keyRows}</div>
    </div>`;
  }).join("");
}

function drmTypeLabel(entry: WvpKeyEntry): string {
  const sid = (entry as Record<string, unknown>).systemId;
  const t = entry.type ?? "";
  if (typeof sid === "string") {
    const norm = sid.replace(/-/g, "").toLowerCase();
    if (norm === "edef8ba979d64acea3c827dcd51d21ed") return "Widevine";
    if (norm === "9a04f07998404286ab92e65be0885f95") return "PlayReady";
  }
  if (t.toLowerCase() === "widevine") return "Widevine";
  if (t.toLowerCase() === "playready") return "PlayReady";
  if (t) return "DRM";
  return "";
}

async function loadDrmKeys() {
  setDrmStatus("checking", "Loading…");
  try {
    const all = await browserApi.storage.local.get(null);
    const entries = Object.values(all)
      .filter((v): v is WvpKeyEntry =>
        typeof v === "object" && v !== null && ("keys" in v || "type" in v),
      )
      .sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0));
    renderDrmKeys(entries);
  } catch {
    setDrmStatus("disconnected", "Error");
    drmKeyList.innerHTML = `<div class="empty-state"><p>Could not load keys</p></div>`;
  }
}

function initDrmTab() {
  drmHistoryBtn.addEventListener("click", () => {
    const url = chrome.runtime.getURL("widevine/ui/history.html");
    void browserApi.tabs.create({ url });
    window.close();
  });

  drmClearBtn.addEventListener("click", () => {
    void (async () => {
      const all = await browserApi.storage.local.get(null);
      const keysToRemove = Object.keys(all).filter((k) => {
        const v = all[k];
        return typeof v === "object" && v !== null && ("keys" in v || "type" in v);
      });
      if (keysToRemove.length) await browserApi.storage.local.remove(keysToRemove);
      await loadDrmKeys();
    })();
  });

  // Listen for new keys arriving while popup is open
  chrome.storage.onChanged.addListener((_changes, areaName) => {
    if (areaName === "local") {
      const drmTab = document.querySelector<HTMLButtonElement>('[data-tab="drm"]');
      if (drmTab && drmTab.classList.contains("tab-active")) {
        void loadDrmKeys();
      }
    }
  });
}

void init();
