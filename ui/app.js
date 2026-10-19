// Tauri API Bridge
const invoke = window.__TAURI__ ? window.__TAURI__.core.invoke : async () => {};
const listen = window.__TAURI__ ? window.__TAURI__.event.listen : async () => {};

// Elements
const langSelect = document.getElementById("langSelect");
const cookieInput = document.getElementById("cookieInput");
const apiKeyInput = document.getElementById("apiKeyInput");
const rememberCreds = document.getElementById("rememberCreds");
const toggleCookieBtn = document.getElementById("toggleCookieBtn");
const toggleApiKeyBtn = document.getElementById("toggleApiKeyBtn");

const autoModeToggle = document.getElementById("autoModeToggle");
const autoModeSwitchLabel = document.getElementById("autoModeSwitchLabel");
const autoModeBanner = document.getElementById("autoModeBanner");
const autoDetectBtn = document.getElementById("autoDetectBtn");
const autoGenApiKeyBtn = document.getElementById("autoGenApiKeyBtn");
const openCloudPageBtn = document.getElementById("openCloudPageBtn");
const accountBadge = document.getElementById("accountBadge");
const accountNameText = document.getElementById("accountNameText");

const fileInput = document.getElementById("fileInput");
const browseFileBtn = document.getElementById("browseFileBtn");
const outputInput = document.getElementById("outputInput");
const browseOutputBtn = document.getElementById("browseOutputBtn");
const clearOutputBtn = document.getElementById("clearOutputBtn");

const groupIdInput = document.getElementById("groupIdInput");
const threadsSlider = document.getElementById("threadsSlider");
const threadsValue = document.getElementById("threadsValue");
const unformattedIds = document.getElementById("unformattedIds");
const keepNames = document.getElementById("keepNames");
const dryRun = document.getElementById("dryRun");

const startBtn = document.getElementById("startBtn");
const startBtnText = document.getElementById("startBtnText");
const connectionBadge = document.getElementById("connectionBadge");
const connectionBadgeText = document.getElementById("connectionBadgeText");

const animCountStat = document.getElementById("animCountStat");
const audioCountStat = document.getElementById("audioCountStat");
const replacedCountStat = document.getElementById("replacedCountStat");
const currentStageStat = document.getElementById("currentStageStat");

const progressBarFill = document.getElementById("progressBarFill");
const progressStatusText = document.getElementById("progressStatusText");
const progressPercentText = document.getElementById("progressPercentText");

const consoleBody = document.getElementById("consoleBody");
const copyLogsBtn = document.getElementById("copyLogsBtn");
const clearLogsBtn = document.getElementById("clearLogsBtn");

// Local Storage Keys
const STORAGE_COOKIE = "rar_cookie";
const STORAGE_APIKEY = "rar_apikey";
const STORAGE_REMEMBER = "rar_remember";
const STORAGE_AUTOMODE = "rar_automode";
const STORAGE_LANG = "rar_lang";

// Multi-Language (i18n) Engine
let currentLang = "tr";
let lastRawStage = "Ready";

function getDict() {
  if (typeof window !== "undefined" && window.translations) return window.translations;
  if (typeof globalThis !== "undefined" && globalThis.translations) return globalThis.translations;
  if (typeof translations !== "undefined") return translations;
  return {};
}

function t(key) {
  const dict = getDict();
  if (dict[currentLang] && dict[currentLang][key]) {
    return dict[currentLang][key];
  }
  if (dict.en && dict.en[key]) {
    return dict.en[key];
  }
  if (dict.tr && dict.tr[key]) {
    return dict.tr[key];
  }
  return key;
}

function translateStage(stage) {
  if (!stage) return t("idle");
  switch (stage) {
    case "Ready":
    case "Ready to start": return t("ready");
    case "Reading Place File": return t("stageReading");
    case "Scanning Workspace": return t("stageWorkspace");
    case "Scanning Scripts": return t("stageScripts");
    case "Reuploading Assets": return t("stageUploading");
    case "Updating Place Data": return t("stageUpdating");
    case "Saving Place File": return t("stageSaving");
    case "Completed Successfully":
    case "Completed":
    case "Completed (Dry Run)":
    case "Finished": return t("stageFinished");
    case "Initializing":
    case "Starting...": return t("stageInit");
    case "Error": return t("stageError");
    case "Idle": return t("idle");
    default: return stage;
  }
}

function setLanguage(lang) {
  const dict = getDict();
  if (!dict[lang]) {
    lang = "en";
  }
  currentLang = lang;
  try {
    localStorage.setItem(STORAGE_LANG, lang);
  } catch (e) {
    console.warn("Could not save language preference:", e);
  }

  // Set RTL or LTR layout
  const isRtl = (lang === "ar");
  document.documentElement.setAttribute("dir", isRtl ? "rtl" : "ltr");
  document.documentElement.setAttribute("lang", lang);

  if (langSelect && langSelect.value !== lang) {
    langSelect.value = lang;
  }

  // Update text contents
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    const val = t(key);
    if (val !== undefined && val !== null && val !== "") {
      el.textContent = val;
    }
  });

  // Update placeholders
  document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    const key = el.getAttribute("data-i18n-placeholder");
    const val = t(key);
    if (val !== undefined && val !== null && val !== "") {
      el.placeholder = val;
    }
  });

  // Update dynamic titles/tooltips
  if (autoModeSwitchLabel) {
    autoModeSwitchLabel.title = t("autoModeTooltip");
  }

  // Update current stage label
  if (lastRawStage) {
    const localized = translateStage(lastRawStage);
    if (progressStatusText) progressStatusText.textContent = localized;
    if (currentStageStat) currentStageStat.textContent = localized;
  }
}
window.setLanguage = setLanguage;

function initLanguage() {
  const dict = getDict();
  let savedLang = null;
  try {
    savedLang = localStorage.getItem(STORAGE_LANG);
  } catch (e) {
    console.warn("LocalStorage getItem failed:", e);
  }
  if (savedLang && dict[savedLang]) {
    setLanguage(savedLang);
    return;
  }

  // Detect from browser/system language
  let navLang = "en";
  try {
    navLang = (navigator.language || navigator.userLanguage || "en").toLowerCase();
  } catch (e) {}
  const langPrefix = navLang.split("-")[0];

  if (dict[langPrefix]) {
    setLanguage(langPrefix);
  } else {
    // Default to Turkish if Turkish locale, otherwise English
    setLanguage(langPrefix === "tr" ? "tr" : "en");
  }
}

if (langSelect) {
  langSelect.addEventListener("change", (e) => {
    setLanguage(e.target.value);
  });
  langSelect.addEventListener("input", (e) => {
    setLanguage(e.target.value);
  });
}

// Auto-Mode UI Update
function updateAutoModeUI() {
  const isAuto = autoModeToggle.checked;
  autoModeBanner.style.display = isAuto ? "flex" : "none";
}

autoModeToggle.addEventListener("change", () => {
  updateAutoModeUI();
  if (autoModeToggle.checked && !cookieInput.value.trim()) {
    triggerAutoDetectCookie(true);
  }
});

// Detect Roblox Studio Session
async function triggerAutoDetectCookie(silent = false) {
  try {
    if (!silent) {
      addLog("info", "Scanning Windows Credential Manager for Roblox Studio login...");
    }
    const res = await invoke("auto_detect_cookie");
    if (res && res.found) {
      cookieInput.value = res.cookie;
      accountBadge.style.display = "flex";
      const label = res.username
        ? `Studio: ${res.username}${res.user_id ? ` (${res.user_id})` : ""}`
        : "Studio Session Connected";
      accountNameText.textContent = label;
      addLog("success", res.message);
      saveCredentials();
    } else if (!silent && res) {
      addLog("warn", res.message);
    }
  } catch (err) {
    if (!silent) {
      addLog("error", `Auto-detect failed: ${err}`);
    }
  }
}

autoDetectBtn.addEventListener("click", () => {
  triggerAutoDetectCookie(false);
});

// Auto-Generate Open Cloud API Key
autoGenApiKeyBtn.addEventListener("click", async () => {
  try {
    autoGenApiKeyBtn.disabled = true;
    addLog("info", "Attempting to auto-generate Open Cloud API Key...");
    const res = await invoke("auto_generate_api_key", {
      cookie: cookieInput.value.trim(),
    });
    if (res && res.created && res.api_key) {
      apiKeyInput.value = res.api_key;
      addLog("success", res.message);
      saveCredentials();
    } else if (res) {
      addLog("info", res.message);
    }
  } catch (err) {
    addLog("warn", `Auto-Key notice: ${err}`);
  } finally {
    autoGenApiKeyBtn.disabled = false;
  }
});

// Open Creator Dashboard Page
openCloudPageBtn.addEventListener("click", async () => {
  try {
    await invoke("open_browser_url", {
      url: "https://create.roblox.com/dashboard/credentials?activeTab=ApiKeysTab",
    });
    addLog("info", "Opened Roblox Creator Dashboard API Keys page in browser.");
  } catch (err) {
    addLog("error", `Could not open browser: ${err}`);
  }
});

// Load Saved Credentials
function loadSavedCredentials() {
  const remember = localStorage.getItem(STORAGE_REMEMBER) !== "false";
  rememberCreds.checked = remember;

  const savedAuto = localStorage.getItem(STORAGE_AUTOMODE);
  if (savedAuto !== null) {
    autoModeToggle.checked = savedAuto === "true";
  }
  updateAutoModeUI();

  if (remember) {
    const savedCookie = localStorage.getItem(STORAGE_COOKIE) || "";
    const savedApiKey = localStorage.getItem(STORAGE_APIKEY) || "";
    if (savedCookie) cookieInput.value = savedCookie;
    if (savedApiKey) apiKeyInput.value = savedApiKey;
  }
}

// Save Credentials
function saveCredentials() {
  localStorage.setItem(STORAGE_AUTOMODE, autoModeToggle.checked ? "true" : "false");
  if (rememberCreds.checked) {
    localStorage.setItem(STORAGE_REMEMBER, "true");
    localStorage.setItem(STORAGE_COOKIE, cookieInput.value.trim());
    localStorage.setItem(STORAGE_APIKEY, apiKeyInput.value.trim());
  } else {
    localStorage.setItem(STORAGE_REMEMBER, "false");
    localStorage.removeItem(STORAGE_COOKIE);
    localStorage.removeItem(STORAGE_APIKEY);
  }
}

rememberCreds.addEventListener("change", saveCredentials);

// Toggle Password Visibility
toggleCookieBtn.addEventListener("click", () => {
  cookieInput.type = cookieInput.type === "password" ? "text" : "password";
});

toggleApiKeyBtn.addEventListener("click", () => {
  apiKeyInput.type = apiKeyInput.type === "password" ? "text" : "password";
});

// Threads Slider
threadsSlider.addEventListener("input", (e) => {
  threadsValue.textContent = e.target.value;
});

// File Pickers
browseFileBtn.addEventListener("click", async () => {
  try {
    const selected = await invoke("pick_file");
    if (selected) {
      fileInput.value = selected;
      addLog("info", `Selected place file: ${selected}`);
    }
  } catch (err) {
    addLog("error", `Failed to pick file: ${err}`);
  }
});

browseOutputBtn.addEventListener("click", async () => {
  try {
    let defName = null;
    if (fileInput.value) {
      const parts = fileInput.value.split(/[\\/]/);
      const fileName = parts[parts.length - 1];
      defName = fileName.replace(/\.rbxlx?$/i, "_replaced.rbxl");
    }
    const selected = await invoke("pick_output_file", { defaultName: defName });
    if (selected) {
      outputInput.value = selected;
      addLog("info", `Output path set to: ${selected}`);
    }
  } catch (err) {
    addLog("error", `Failed to pick output: ${err}`);
  }
});

clearOutputBtn.addEventListener("click", () => {
  outputInput.value = "";
});

// Console Log Handler (capped at 500 entries to prevent DOM memory bloat on massive places)
const MAX_LOG_LINES = 500;
function addLog(level, message) {
  const line = document.createElement("div");
  line.className = `log-line log-${level}`;
  const time = new Date().toLocaleTimeString();
  line.textContent = `[${time}] [${level.toUpperCase()}] ${message}`;
  consoleBody.appendChild(line);
  while (consoleBody.children.length > MAX_LOG_LINES) {
    consoleBody.removeChild(consoleBody.firstChild);
  }
  consoleBody.scrollTop = consoleBody.scrollHeight;
}

clearLogsBtn.addEventListener("click", () => {
  consoleBody.innerHTML = "";
  addLog("info", t("clearedLogs"));
});

copyLogsBtn.addEventListener("click", () => {
  const text = Array.from(consoleBody.querySelectorAll(".log-line"))
    .map((el) => el.textContent)
    .join("\n");
  navigator.clipboard.writeText(text).then(() => {
    addLog("success", t("copiedLogs"));
  });
});

// Progress Handler
function setProgress(percent, stageName) {
  progressBarFill.style.width = `${percent}%`;
  progressPercentText.textContent = `${percent}%`;
  if (stageName) {
    lastRawStage = stageName;
    const localizedStage = translateStage(stageName);
    progressStatusText.textContent = localizedStage;
    currentStageStat.textContent = localizedStage;
  }
}

// Start Replacement Action
startBtn.addEventListener("click", async () => {
  const cookie = cookieInput.value.trim();
  const apiKey = apiKeyInput.value.trim();
  const file = fileInput.value.trim();
  const isAuto = autoModeToggle.checked;

  if (!isAuto && !cookie) {
    alert(t("alertCookie"));
    cookieInput.focus();
    return;
  }
  if (!file) {
    alert(t("alertFile"));
    browseFileBtn.focus();
    return;
  }

  saveCredentials();

  // Reset Stats
  animCountStat.textContent = "0";
  audioCountStat.textContent = "0";
  replacedCountStat.textContent = "0";
  setProgress(0, "Starting...");
  addLog("info", "Initiating process...");

  // UI Busy State
  startBtn.disabled = true;
  startBtn.innerHTML = `<span>${t("processing")}</span>`;
  connectionBadge.innerHTML = `<span class="status-dot" style="background:#f59e0b;box-shadow:0 0 8px #f59e0b;"></span> <span id="connectionBadgeText">${t("working")}</span>`;

  const config = {
    cookie: isAuto && !cookie ? "" : cookie,
    api_key: apiKey,
    file,
    output: outputInput.value.trim() || null,
    group: groupIdInput.value.trim() ? parseInt(groupIdInput.value.trim(), 10) : null,
    threads: parseInt(threadsSlider.value, 10),
    unformatted_ids: unformattedIds.checked,
    dry_run: dryRun.checked,
    keep_names: keepNames.checked,
  };

  try {
    const result = await invoke("run_replacement", { config });

    animCountStat.textContent = result.animations_count;
    audioCountStat.textContent = result.audios_count;
    replacedCountStat.textContent = result.replaced_count;

    if (result.success) {
      addLog("success", result.message);
      setProgress(100, "Finished");
      connectionBadge.innerHTML = `<span class="status-dot"></span> <span id="connectionBadgeText">${t("completed")}</span>`;
    } else {
      addLog("error", result.message);
      connectionBadge.innerHTML = `<span class="status-dot" style="background:#ef4444;box-shadow:0 0 8px #ef4444;"></span> <span id="connectionBadgeText">${t("failed")}</span>`;
    }
  } catch (err) {
    addLog("error", `Process error: ${err}`);
    setProgress(0, "Error");
    connectionBadge.innerHTML = `<span class="status-dot" style="background:#ef4444;box-shadow:0 0 8px #ef4444;"></span> <span id="connectionBadgeText">${t("error")}</span>`;
  } finally {
    startBtn.disabled = false;
    startBtn.innerHTML = `
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" class="btn-icon-svg">
        <polygon points="5 3 19 12 5 21 5 3"></polygon>
      </svg>
      <span id="startBtnText" data-i18n="startBtn">${t("startBtn")}</span>
    `;
  }
});

// Listen to Tauri Backend Events
async function initEventListeners() {
  if (!window.__TAURI__) return;

  await listen("app_log", (event) => {
    const { level, message } = event.payload;
    addLog(level, message);
  });

  await listen("app_progress", (event) => {
    const { current, total, stage } = event.payload;
    const percent = Math.min(100, Math.round((current / (total || 1)) * 100));
    setProgress(percent, stage);
  });
}

// Startup
initLanguage();
loadSavedCredentials();
initEventListeners().then(() => {
  if (autoModeToggle.checked) {
    triggerAutoDetectCookie(true);
  }
}).catch((err) => console.error("Event listener error:", err));
