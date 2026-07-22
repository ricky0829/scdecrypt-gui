import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

// Elements
const folderIdInput = document.getElementById("folder-id") as HTMLInputElement;
const passwordInput = document.getElementById("password") as HTMLInputElement;
const sourcePathInput = document.getElementById("source-path") as HTMLInputElement;
const destPathInput = document.getElementById("dest-path") as HTMLInputElement;
const fileListDisplay = document.getElementById("file-list-display") as HTMLInputElement;
const btnDecrypt = document.getElementById("btn-decrypt") as HTMLButtonElement;
const btnClearLog = document.getElementById("btn-clear-log") as HTMLButtonElement;
const modalAbout = document.getElementById("modal-about") as HTMLDivElement;
const logArea = document.getElementById("log-area") as HTMLDivElement;
const logSection = document.getElementById("log-section") as HTMLDivElement;
const statusBadge = document.getElementById("status-badge") as HTMLDivElement;
const modeFolderDiv = document.getElementById("mode-folder") as HTMLDivElement;
const modeFileDiv = document.getElementById("mode-file") as HTMLDivElement;
const syncthingVersionEl = document.getElementById("syncthing-version") as HTMLSpanElement;

let syncthingPath: string | null = null;
let currentMode: "folder" | "file" = "folder";
let logVisible = true;
let savedLogHeight = 0;
let selectedFiles: string[] = [];

const appWindow = getCurrentWindow();

// Custom in-app confirm dialog (always centered in window, unlike native ask())
const modalConfirm = document.getElementById("modal-confirm-download") as HTMLDivElement;
function showDownloadConfirm(): Promise<boolean> {
  return new Promise((resolve) => {
    modalConfirm.classList.remove("hidden");
    const done = (result: boolean) => {
      modalConfirm.classList.add("hidden");
      resolve(result);
    };
    document.getElementById("btn-confirm-download")!.onclick = () => done(true);
    document.getElementById("btn-cancel-download")!.onclick = () => done(false);
  });
}

// ===== Init =====
async function init() {
  try {
    const path = await invoke<string | null>("find_syncthing");
    if (path) {
      syncthingPath = path;
      addLog(`已找到 syncthing.exe: ${path}`, "info");
      loadSyncthingVersion();
    } else {
      addLog("未找到 syncthing.exe", "error");
      // Ask user whether to download (in-app modal, centered in window)
      const yes = await showDownloadConfirm();
      if (yes) openUpdateModal();
    }
  } catch (e) {
    addLog(`初始化失败: ${e}`, "error");
  }
}

async function loadSyncthingVersion() {
  try {
    const ver = await invoke<string>("get_syncthing_version");
    syncthingVersionEl.textContent = `Syncthing 版本: ${ver}`;
  } catch {
    syncthingVersionEl.textContent = "Syncthing 版本: 未检测到";
  }
}

// ===== Log =====
function addLog(text: string, type: "info" | "cmd" | "error" | "success" | "normal" = "normal") {
  const line = document.createElement("div");
  line.className = `log-line log-${type}`;
  line.textContent = text;
  logArea.appendChild(line);
  logArea.scrollTop = logArea.scrollHeight;
}

// ===== Toggle Log + Resize Window =====
async function toggleLog() {
  const btn = document.getElementById("btn-toggle-log")!;

  if (logVisible) {
    // Step 1: measure + hide DOM first (always takes effect)
    savedLogHeight = logSection.offsetHeight || 180;
    logSection.classList.add("hidden");
    // Step 2: shrink window (independent, errors visible)
    try {
      const inner = await appWindow.innerSize();
      const factor = await appWindow.scaleFactor();
      const logicalH = Math.round(inner.height / factor);
      const newH = Math.max(260, logicalH - savedLogHeight);
      await appWindow.setSize(new LogicalSize(Math.round(inner.width / factor), newH));
    } catch (e) {
      addLog(`[警告] 窗口缩放失败: ${e}`, "error");
    }
  } else {
    // Step 1: show DOM first
    logSection.classList.remove("hidden");
    // Step 2: grow window back
    try {
      const inner = await appWindow.innerSize();
      const factor = await appWindow.scaleFactor();
      const logicalH = Math.round(inner.height / factor);
      const growH = savedLogHeight > 0 ? savedLogHeight : 180;
      await appWindow.setSize(new LogicalSize(Math.round(inner.width / factor), logicalH + growH));
    } catch (e) {
      addLog(`[警告] 窗口缩放失败: ${e}`, "error");
    }
  }

  logVisible = !logVisible;
  btn.textContent = logVisible ? "隐藏日志" : "显示日志";
}

// ===== Mode Switch =====
function setMode(mode: "folder" | "file") {
  currentMode = mode;
  document.querySelectorAll(".mode-btn").forEach((btn) => {
    btn.classList.toggle("active", (btn as HTMLElement).dataset.mode === mode);
  });
  modeFolderDiv.classList.toggle("hidden", mode !== "folder");
  modeFileDiv.classList.toggle("hidden", mode !== "file");
}

// ===== Browse =====
async function browseSourceFolder() {
  const selected = await open({ directory: true, multiple: false });
  if (selected) sourcePathInput.value = selected as string;
}

async function browseDestFolder() {
  const selected = await open({ directory: true, multiple: false });
  if (selected) destPathInput.value = selected as string;
}

async function browseFiles() {
  const selected = await open({ directory: false, multiple: true });
  if (selected && Array.isArray(selected)) {
    selectedFiles = selected as string[];
    if (selectedFiles.length === 1) {
      fileListDisplay.value = selectedFiles[0];
    } else {
      fileListDisplay.value = `已选择 ${selectedFiles.length} 个文件`;
    }
  }
}

// ===== Decrypt =====
async function startDecrypt() {
  const folderId = folderIdInput.value.trim();
  const password = passwordInput.value.trim();
  const dest = destPathInput.value.trim();

  if (!password) { addLog("[提示] 请输入密码", "error"); return; }
  if (!dest) { addLog("[提示] 请选择输出目录", "error"); return; }

  if (currentMode === "folder") {
    const source = sourcePathInput.value.trim();
    if (!source) { addLog("[提示] 请选择加密目录", "error"); return; }
    if (!syncthingPath) { if (!(await ensureSyncthing())) return; }
    setRunning(true);
    try {
      await invoke("decrypt", { syncthingPath, folderId, password, source, dest });
    } catch (e) { addLog(`[错误] ${e}`, "error"); setRunning(false); }
  } else {
    if (selectedFiles.length === 0) { addLog("[提示] 请选择要解密的文件", "error"); return; }
    if (!folderId) { addLog("[提示] 文件解密模式必须填写 Folder ID", "error"); return; }
    if (!syncthingPath) { if (!(await ensureSyncthing())) return; }
    setRunning(true);
    try {
      await invoke("decrypt_files", { syncthingPath, folderId, password, files: selectedFiles, dest });
    } catch (e) { addLog(`[错误] ${e}`, "error"); setRunning(false); }
  }
}

async function ensureSyncthing(): Promise<boolean> {
  const path = await invoke<string | null>("find_syncthing");
  if (path) { syncthingPath = path; return true; }
  addLog("[错误] 未找到 syncthing.exe，请先通过 工具→更新Syncthing 下载", "error");
  return false;
}

function setRunning(running: boolean) {
  btnDecrypt.disabled = running;
  if (running) {
    statusBadge.className = "status-badge running";
    statusBadge.textContent = "解密中...";
    statusBadge.classList.remove("hidden");
  }
}

// ===== Update Syncthing =====
const modalUpdate = document.getElementById("modal-update") as HTMLDivElement;
const customProxyRow = document.getElementById("row-custom-proxy") as HTMLDivElement;
const customProxyInput = document.getElementById("custom-proxy") as HTMLInputElement;
const updateOptionsDiv = document.getElementById("update-options") as HTMLDivElement;
const updateProgressArea = document.getElementById("update-progress-area") as HTMLDivElement;
const updateProgressBar = document.getElementById("update-progress-bar") as HTMLDivElement;
const updateProgressText = document.getElementById("update-progress-text") as HTMLDivElement;
const btnStartUpdate = document.getElementById("btn-start-update") as HTMLButtonElement;
const btnCancelUpdate = document.getElementById("btn-cancel-update") as HTMLButtonElement;
const chkMirror = document.getElementById("chk-mirror") as HTMLInputElement;
const chkSysProxy = document.getElementById("chk-sys-proxy") as HTMLInputElement;
const chkCustomProxy = document.getElementById("chk-custom-proxy") as HTMLInputElement;
const verLocal = document.getElementById("ver-local") as HTMLSpanElement;
const verLatest = document.getElementById("ver-latest") as HTMLSpanElement;
const updateResultArea = document.getElementById("update-result-area") as HTMLDivElement;
const updateResultIcon = document.getElementById("update-result-icon") as HTMLDivElement;
const updateResultText = document.getElementById("update-result-text") as HTMLDivElement;
let updateRunning = false;

function openUpdateModal() {
  modalUpdate.classList.remove("hidden");
  // Fill local version info
  invoke<string>("get_syncthing_version")
    .then((ver) => { verLocal.textContent = `本地版本：${ver}`; })
    .catch(() => { verLocal.textContent = "本地版本：未检测到"; });
  verLatest.textContent = "最新版本：待查询";
}

function resetUpdateModal() {
  updateRunning = false;
  updateOptionsDiv.classList.remove("hidden");
  updateProgressArea.classList.add("hidden");
  updateResultArea.classList.add("hidden");
  updateProgressBar.style.width = "0%";
  updateProgressText.textContent = "准备中...";
  btnStartUpdate.classList.remove("hidden");
  btnStartUpdate.disabled = false;
  btnCancelUpdate.textContent = "取消";
}

function showUpdateResult(success: boolean, message: string) {
  updateProgressArea.classList.add("hidden");
  updateResultArea.classList.remove("hidden");
  updateResultIcon.textContent = success ? "✓" : "✗";
  updateResultIcon.className = `result-icon ${success ? "success" : "error"}`;
  updateResultText.textContent = message;
  btnCancelUpdate.textContent = "关闭";
  updateRunning = false;
}

function startUpdate() {
  const useMirror = chkMirror.checked;
  const proxyMode = chkCustomProxy.checked ? "custom" : (chkSysProxy.checked ? "system" : "none");
  const proxyAddr = proxyMode === "custom" ? customProxyInput.value.trim() : "";
  // Switch modal to progress mode (stay open)
  updateRunning = true;
  updateOptionsDiv.classList.add("hidden");
  updateProgressArea.classList.remove("hidden");
  updateProgressBar.style.width = "0%";
  updateProgressText.textContent = "正在查询最新版本...";
  btnStartUpdate.classList.add("hidden");
  btnCancelUpdate.textContent = "取消更新";
  addLog("[信息] 正在检查 Syncthing 更新...", "info");
  invoke("update_syncthing", {
    useMirror,
    proxyMode,
    proxyAddr,
  }).catch((e) => addLog(`[错误] 更新失败: ${e}`, "error"));
}

function handleUpdateCancel() {
  if (updateRunning) {
    // Cancel in-progress update: close modal immediately, backend aborts ASAP
    invoke("cancel_update").catch(() => {});
    addLog("[信息] 正在取消更新...", "info");
  }
  // Always close immediately (no waiting for backend)
  resetUpdateModal();
  modalUpdate.classList.add("hidden");
}

// Update modal: checkbox interactions
// System proxy and custom proxy are mutually exclusive
chkSysProxy.addEventListener("change", () => {
  if (chkSysProxy.checked) chkCustomProxy.checked = false;
  customProxyRow.classList.add("hidden");
});
chkCustomProxy.addEventListener("change", () => {
  if (chkCustomProxy.checked) chkSysProxy.checked = false;
  customProxyRow.classList.toggle("hidden", !chkCustomProxy.checked);
});
btnStartUpdate.addEventListener("click", startUpdate);
btnCancelUpdate.addEventListener("click", handleUpdateCancel);

// ===== Menu Bar =====
function setupMenus() {
  document.querySelectorAll(".menu-item").forEach((item) => {
    const trigger = item.querySelector("span")!;
    const dropdown = item.querySelector(".menu-dropdown")!;
    trigger.addEventListener("click", (e) => {
      e.stopPropagation();
      const isHidden = dropdown.classList.contains("hidden");
      closeAllMenus();
      if (isHidden) dropdown.classList.remove("hidden");
    });
  });
  document.addEventListener("click", closeAllMenus);
}

function closeAllMenus() {
  document.querySelectorAll(".menu-dropdown").forEach((d) => d.classList.add("hidden"));
}

// ===== Event Listeners =====
async function setupListeners() {
  await listen<{ line: string }>("decrypt-log", (event) => {
    const line = event.payload.line;
    if (line.startsWith("[CMD]")) addLog(line, "cmd");
    else if (line.startsWith("[错误]") || line.startsWith("[stderr]")) addLog(line, "error");
    else if (line.startsWith("[完成]")) addLog(line, "success");
    else if (line.startsWith("[信息]")) addLog(line, "info");
    else addLog(line);
  });

  await listen<{ success: boolean; code: number }>("decrypt-status", (event) => {
    const { success, code } = event.payload;
    btnDecrypt.disabled = false;
    if (success) {
      statusBadge.className = "status-badge success";
      statusBadge.textContent = "解密成功";
    } else {
      statusBadge.className = "status-badge error";
      statusBadge.textContent = "解密失败";
      addLog(`[失败] 退出码: ${code}`, "error");
    }
  });

  await listen<{ line: string }>("update-log", (event) => {
    addLog(event.payload.line, "info");
    // Parse latest version into modal version display
    const verMatch = event.payload.line.match(/最新版本: (v[\d.]+)/);
    if (verMatch) {
      verLatest.textContent = `最新版本：${verMatch[1]}`;
    }
    // Also reflect status in modal progress text while updating
    if (updateRunning && !modalUpdate.classList.contains("hidden")) {
      const line = event.payload.line;
      if (line.startsWith("[信息]") || line.startsWith("[完成]")) {
        const text = line.replace(/^\[(信息|完成)\]\s*/, "");
        if (!text.includes("正在下载")) {
          updateProgressText.textContent = text;
        }
      }
    }
  });

  // Download progress: update modal progress bar
  await listen<{ percent: number; downloaded_mb: number; total_mb: number; speed_mbps: number; done: boolean }>("update-progress", (event) => {
    const p = event.payload;
    if (p.done) {
      updateProgressBar.style.width = "100%";
      return;
    }
    updateProgressBar.style.width = `${p.percent.toFixed(1)}%`;
    updateProgressText.textContent = p.total_mb > 0
      ? `${p.percent.toFixed(1)}%  ${p.downloaded_mb.toFixed(1)}/${p.total_mb.toFixed(1)} MB  ${p.speed_mbps.toFixed(2)} MB/s`
      : `已下载 ${p.downloaded_mb.toFixed(1)} MB  ${p.speed_mbps.toFixed(2)} MB/s`;
  });

  await listen<{ success: boolean; message: string }>("update-status", (event) => {
    const { success, message } = event.payload;
    if (modalUpdate.classList.contains("hidden")) {
      // Modal already closed by user (cancelled) - just log
      if (success) {
        addLog(`[完成] ${message}`, "success");
      } else {
        addLog(`[信息] 更新已终止: ${message}`, "info");
      }
    } else {
      // Modal still open - show result page (user closes manually)
      showUpdateResult(success, message);
      if (success) {
        addLog(`[完成] ${message}`, "success");
      } else {
        addLog(`[失败] 更新失败: ${message}`, "error");
      }
    }
    if (success) {
      loadSyncthingVersion();
      invoke<string | null>("find_syncthing").then((p) => { syncthingPath = p; });
    }
  });
}

// ===== Bindings =====
document.querySelectorAll(".mode-btn").forEach((btn) => {
  btn.addEventListener("click", () => setMode((btn as HTMLElement).dataset.mode as "folder" | "file"));
});

// JS-managed hover: event delegation with mouseover (bulletproof against WebView2 stuck :hover)
// Modal buttons are excluded entirely - they have no hover effect (CSS enforced)
document.addEventListener("mouseover", (e) => {
  const target = (e.target as HTMLElement).closest?.(".mode-btn") as HTMLElement | null;
  const valid = target && !target.closest(".modal") ? target : null;
  document.querySelectorAll(".mode-btn.hovered").forEach((el) => {
    if (el !== valid) el.classList.remove("hovered");
  });
  if (valid) valid.classList.add("hovered");
});
document.addEventListener("mouseleave", () => {
  document.querySelectorAll(".mode-btn.hovered").forEach((el) => el.classList.remove("hovered"));
}, true);

document.getElementById("btn-browse-source")!.addEventListener("click", browseSourceFolder);
document.getElementById("btn-browse-dest")!.addEventListener("click", browseDestFolder);
document.getElementById("btn-browse-files")!.addEventListener("click", browseFiles);
btnDecrypt.addEventListener("click", startDecrypt);
btnClearLog.addEventListener("click", () => { logArea.innerHTML = ""; });
document.getElementById("btn-export-log")!.addEventListener("click", exportLog);
document.getElementById("btn-toggle-log")!.addEventListener("click", toggleLog);
document.getElementById("btn-update-syncthing")!.addEventListener("click", openUpdateModal);
document.getElementById("btn-about")!.addEventListener("click", () => modalAbout.classList.remove("hidden"));
document.getElementById("btn-close-about")!.addEventListener("click", () => modalAbout.classList.add("hidden"));
modalAbout.addEventListener("click", (e) => { if (e.target === modalAbout) modalAbout.classList.add("hidden"); });

// Open external links (via tauri-plugin-opener, no console window)
document.querySelectorAll(".ext-link").forEach((link) => {
  link.addEventListener("click", (e) => {
    e.preventDefault();
    const url = (link as HTMLElement).dataset.url;
    if (url) openUrl(url).catch(() => {});
  });
});

// Init
setupMenus();
setupListeners();
init();

// ===== Export Log =====
async function exportLog() {
  const lines = Array.from(logArea.querySelectorAll(".log-line")).map(el => el.textContent || "");
  if (lines.length === 0) { addLog("[提示] 日志为空", "info"); return; }
  const path = await save({ defaultPath: "decrypt_log.txt", filters: [{ name: "Text", extensions: ["txt", "log"] }] });
  if (path) {
    try {
      await invoke("save_file", { path, content: lines.join("\n") });
      addLog(`[信息] 日志已导出: ${path}`, "success");
    } catch (e) {
      addLog(`[错误] 导出失败: ${e}`, "error");
    }
  }
}
