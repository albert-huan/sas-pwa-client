// SAS PWA 客户端设置页：维护 SAS 站点配置（增删改）并通过 Tauri 命令持久化 / 打开窗口。
// 依赖 withGlobalTauri=true 注入的全局 __TAURI__，无需 npm 依赖。
// 文案多语种：见 ./i18n.js（t / applyI18n / getLang / setLang …）。

import { t, applyI18n, getLang, setLang, langOptions } from "./i18n.js";

const tauri = typeof window !== "undefined" ? window.__TAURI__ : undefined;
const core = tauri && tauri.core;
const ENABLED = !!(core && typeof core.invoke === "function");

const el = (id) => document.getElementById(id);
const dom = {
  version: el("version"),
  list: el("site-list"),
  status: el("status"),
  toast: el("toast"),
  envWarn: el("env-warn"),
  form: el("form"),
  formTitle: el("editor-title"),
  id: el("f-id"),
  name: el("f-name"),
  url: el("f-url"),
  keep: el("f-keep"),
  frameless: el("f-frameless"),
  isDefault: el("f-default"),
  hint: el("default-hint"),
  user: el("f-user"),
  pass: el("f-pass"),
  autologin: el("f-autologin"),
  credHint: el("cred-hint"),
  clearCred: el("btn-clear-cred"),
  pulse: el("f-pulse"),
  new: el("btn-new"),
  save: el("btn-save"),
  saveOpen: el("btn-save-open"),
  del: el("btn-delete"),
  cancel: el("btn-cancel"),
  close: el("btn-close"),
  checkUpdate: el("btn-check-update"),
  lang: el("lang-select"),
  theme: el("theme-select"),
  linuxCard: el("linux-card"),
  render: el("f-render"),
  devtools: el("f-devtools"),
};

let state = {
  sites: [],
  draft: null,
  editingIndex: -1,
  configPath: "",
  version: "",
  cred: null,
  uiTheme: "system",
  // 渲染模式与平台：Linux 才有这个设置（改的是 WebKitGTK 的渲染路径，重启后生效）。
  renderMode: "auto",
  platform: "",
  // 各站点已开的窗口数（后端查，详见 refreshWindowCounts）：无窗口则没有该键。
  windowCounts: {},
};

function invoke(cmd, args) {
  if (!ENABLED) {
    toast(t("toast.noTauri"), true);
    return Promise.reject(new Error("no tauri"));
  }
  return core.invoke(cmd, args);
}

let toastTimer = 0;
function toast(msg, isError = false) {
  dom.toast.textContent = msg;
  dom.toast.className = isError ? "toast show err" : "toast show";
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    dom.toast.className = "toast";
  }, 2600);
}

function blankDraft() {
  return {
    id: "",
    name: "",
    url: "",
    keep_awake: true,
    pulse_seconds: 120,
    frameless: false,
    default: false,
  };
}

function renderStatus() {
  dom.version.textContent = state.version ? `v${state.version}` : "";
  const count = state.sites.length;
  dom.status.textContent = [
    t("status.count", { n: count }),
    state.configPath ? t("status.config", { path: state.configPath }) : "",
  ]
    .filter(Boolean)
    .join(" · ");

  const def = state.sites.find((s) => s.default);
  dom.hint.textContent = def
    ? t("hint.default", { name: def.name || def.url })
    : t("hint.noDefault");
}

function renderList() {
  dom.list.innerHTML = "";
  if (!state.sites.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = t("list.empty");
    dom.list.appendChild(empty);
    return;
  }
  state.sites.forEach((s, i) => {
    const row = document.createElement("div");
    row.className = "site" + (i === state.editingIndex ? " active" : "");

    const dot = document.createElement("span");
    dot.className = "dot";
    dot.title = s.keep_awake ? t("dot.on") : t("dot.off");
    // 「未开启保活」用主题的中性灰变量，别写死颜色 —— 写死的话亮/暗主题都不跟着变。
    dot.style.background = s.keep_awake ? "var(--ok)" : "var(--muted)";

    const meta = document.createElement("div");
    meta.className = "meta";
    const name = document.createElement("div");
    name.className = "name";
    name.textContent = s.name || t("list.unnamed");
    if (s.default) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = t("badge.default");
      name.appendChild(badge);
    }
    const winCount = state.windowCounts[s.id] || 0;
    if (winCount > 0) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = t("list.windowCount", { n: winCount });
      name.appendChild(badge);
    }
    const url = document.createElement("div");
    url.className = "url";
    url.textContent = s.url;
    meta.append(name, url);

    const acts = document.createElement("div");
    acts.className = "acts";
    acts.append(
      mkBtn(t("list.open"), () => openSite(s.id), "primary sm"),
      // 「新窗口」：为同一站点再开一个窗口，与已有窗口共享同一份登录态（同一账号多开）。
      mkBtn(t("list.newWindow"), () => openSiteNewWindow(s.id), "sm"),
      // 「关闭窗口」：彻底关闭该站点的全部窗口（销毁 WebView）。没有窗口时置灰。
      mkBtn(t("list.closeWindows"), () => closeSiteWindows(s.id), "sm", winCount === 0),
      mkBtn(t("list.edit"), () => startEdit(i), "sm"),
      mkBtn(s.default ? t("list.unsetDefault") : t("list.setDefault"), () => setDefault(i, !s.default), "sm"),
      mkBtn(t("list.delete"), () => removeSite(i), "sm danger")
    );

    row.append(dot, meta, acts);
    dom.list.appendChild(row);
  });
}

function mkBtn(text, onClick, cls = "", disabled = false) {
  const b = document.createElement("button");
  b.className = `btn ${cls}`.trim();
  b.type = "button";
  b.textContent = text;
  b.disabled = !!disabled;
  b.addEventListener("click", onClick);
  return b;
}

function renderForm() {
  const editing = state.editingIndex >= 0;
  dom.formTitle.textContent = editing ? t("editor.edit") : t("editor.new");
  dom.del.hidden = !editing;
  dom.cancel.hidden = !editing;
  if (!editing) {
    fillForm(blankDraft());
  }
}

function fillForm(s) {
  dom.id.value = s.id || "";
  dom.name.value = s.name || "";
  dom.url.value = s.url || "";
  dom.keep.checked = s.keep_awake !== false;
  dom.frameless.checked = !!s.frameless;
  dom.isDefault.checked = !!s.default;
  dom.pulse.value = Number.isFinite(s.pulse_seconds) ? s.pulse_seconds : 120;
  // 凭据单独异步载入：密码永不回显，只显示「是否已保存」。
  dom.pass.value = "";
  void loadCredential(s.id || "");
}

function readForm() {
  return {
    id: dom.id.value.trim(),
    name: dom.name.value.trim(),
    url: dom.url.value.trim(),
    keep_awake: dom.keep.checked,
    pulse_seconds: Math.max(0, Math.min(3600, Number(dom.pulse.value) || 0)),
    frameless: dom.frameless.checked,
    default: dom.isDefault.checked,
  };
}

function startEdit(i) {
  state.editingIndex = i;
  fillForm(state.sites[i]);
  renderList();
  renderForm();
  dom.url.focus();
}

function startNew() {
  state.editingIndex = -1;
  renderList();
  renderForm();
  dom.url.focus();
}

/**
 * 把表单内容合并进站点列表后落盘。
 * @param {boolean} openAfter 保存成功后是否打开该站点窗口
 */
async function persist(openAfter) {
  const draft = readForm();
  if (!draft.url) {
    toast(t("toast.fillUrl"), true);
    dom.url.focus();
    return;
  }
  const sites = state.sites.slice();
  const index = state.editingIndex >= 0 ? state.editingIndex : sites.length;
  // 保存前的那一份，用来判断「保活设置有没有被改动」（它只在建窗时生效，见下方提示）。
  const before = state.editingIndex >= 0 ? state.sites[state.editingIndex] : null;
  if (state.editingIndex >= 0) {
    sites[index] = { ...sites[state.editingIndex], ...draft, id: sites[state.editingIndex].id || draft.id };
  } else {
    sites[index] = draft;
  }
  // 默认登录环境只有一个：本次勾上了，就清掉其它站点的默认标记。
  if (draft.default) {
    sites.forEach((s, n) => {
      if (n !== index) s.default = false;
    });
  }

  try {
    const saved = await invoke("save_config", { sites });
    state.sites = saved;
    state.editingIndex = index;
    renderStatus();
    renderList();
    renderForm();
    // 凭据跟随配置一起保存（密码框留空 = 不改动已保存的密码）。
    await saveCredentialIfNeeded((saved[index] || {}).id || "");
    fillForm(saved[index] || draft);
    // 保活开关与脉冲间隔是「建窗时由注入脚本定下」的，改了必须重开该站点的窗口才生效。
    // 有窗口在跑时必须说清楚，否则用户会以为设置没生效（后端不会再注入一次）。
    const keepChanged =
      !!before &&
      (before.keep_awake !== draft.keep_awake ||
        before.pulse_seconds !== draft.pulse_seconds);
    const running = state.windowCounts[(saved[index] || {}).id] || 0;
    toast(
      keepChanged && running
        ? t("toast.savedReopenNeeded", { n: running })
        : t("toast.saved")
    );
    if (openAfter) {
      await openSite((saved[index] || {}).id);
    }
  } catch (e) {
    toast(t("toast.saveFail", { e }), true);
  }
}

async function openSite(id) {
  if (!id) {
    toast(t("toast.fillUrl"), true);
    return;
  }
  toast(t("toast.opening"));
  try {
    await invoke("open_site", { id });
    void refreshWindowCounts();
  } catch (e) {
    toast(t("toast.openFail", { e }), true);
  }
}

/**
 * 为同一站点再开一个窗口（不聚焦已有窗口）。
 * 窗口共享同一份 cookie / 登录态，所以新窗口打开就是当前登录身份（同一账号多开）。
 */
async function openSiteNewWindow(id) {
  if (!id) {
    toast(t("toast.fillUrl"), true);
    return;
  }
  toast(t("toast.opening"));
  try {
    await invoke("new_site_window", { id });
    void refreshWindowCounts();
  } catch (e) {
    toast(t("toast.openFail", { e }), true);
  }
}

/**
 * 刷新各站点已开的窗口数（后端查询）。窗口是用户在站点窗那边开关的，
 * 设置页这边只能在自己重新拿到焦点时、以及开/关窗口之后主动问一次。
 */
async function refreshWindowCounts() {
  if (!ENABLED) return;
  try {
    state.windowCounts = (await invoke("site_window_counts")) || {};
    renderList();
  } catch (e) {
    /* 查不到就当没有窗口，不影响其它功能 */
  }
}

/**
 * 彻底关闭该站点的全部窗口（销毁 WebView、回收内存）。
 * 「隐藏到托盘继续保活」是另一回事：用站点窗口标题条的 × 或托盘左键。
 */
async function closeSiteWindows(id) {
  if (!id) return;
  if (!(state.windowCounts[id] || 0)) return;
  try {
    const closed = await invoke("close_site_windows", { siteId: id });
    toast(t("toast.windowsClosed", { n: closed }));
  } catch (e) {
    toast(t("toast.closeWinFail", { e }), true);
  }
  refreshWindowCounts();
}

async function removeSite(i) {
  const target = state.sites[i];
  // 站点名下还有窗口时说清楚：后端会连同这些窗口一起销毁。
  // 不销毁的话它们会变成孤儿（托盘的「彻底关闭」对该站点全部置灰 = 再也关不掉）。
  const winCount = state.windowCounts[target.id] || 0;
  const message = winCount
    ? t("confirm.deleteWithWindows", { name: target.name || target.url, n: winCount })
    : t("confirm.delete", { name: target.name || target.url });
  if (!confirm(message)) return;
  const sites = state.sites.filter((_, n) => n !== i);
  try {
    state.sites = await invoke("save_config", { sites });
    renderStatus();
    startNew();
    void refreshWindowCounts();
    toast(t("toast.deleted"));
  } catch (e) {
    toast(t("toast.deleteFail", { e }), true);
  }
}

/**
 * 设置 / 取消默认登录环境（同时只允许一个默认）。
 * @param {number} i 站点下标
 * @param {boolean} value true = 设为默认，false = 取消
 */
async function setDefault(i, value) {
  const sites = state.sites.map((s, n) => ({ ...s, default: n === i && value }));
  try {
    state.sites = await invoke("save_config", { sites });
    renderStatus();
    renderList();
    if (state.editingIndex >= 0) {
      fillForm(state.sites[state.editingIndex] || blankDraft());
    }
    toast(value ? t("toast.setDefault") : t("toast.unsetDefault"));
  } catch (e) {
    toast(t("toast.setFail", { e }), true);
  }
}

/** 载入某站点已保存的凭据（只拿用户名 / 是否已存密码，密码永不回显）。 */
async function loadCredential(siteId) {
  dom.pass.value = "";
  state.cred = null;
  if (ENABLED && siteId) {
    try {
      state.cred = (await invoke("get_credential", { siteId })) || null;
    } catch (e) {
      state.cred = null;
    }
  }
  dom.user.value = (state.cred && state.cred.username) || "";
  dom.autologin.checked = !!(state.cred && state.cred.auto_login);
  renderCredHint();
}

/**
 * 按平台取凭据文案：只有 Windows 走 DPAPI 真加密，其它平台（credentials.rs 的非 Windows
 * 分支）只做十六进制编码，等同明文 —— 不能对它们说「已加密」。
 *
 * platform 还没从 get_config 回来时（启动瞬间那次渲染）按「无 DPAPI」措辞：安全提示宁可
 * 过度，也不能给出虚假的加密保证；load() 完成后会立刻用真实平台重刷。
 */
function credKey(base) {
  return state.platform === "windows" ? base : `${base}Plain`;
}

function renderCredHint() {
  const c = state.cred;
  if (c && c.username) {
    dom.credHint.textContent = c.has_password
      ? t(credKey("cred.savedBoth"), { user: c.username })
      : t("cred.savedUser", { user: c.username });
  } else {
    dom.credHint.textContent = t(credKey("cred.none"));
  }
  dom.clearCred.hidden = !(c && c.username);
}

/**
 * 随配置一起保存凭据：用户名留空且此前存过 → 视为清除。
 * @param {string} siteId 站点 id（新建站点要先保存配置拿到 id）
 */
async function saveCredentialIfNeeded(siteId) {
  if (!ENABLED || !siteId) return;
  const username = dom.user.value.trim();
  if (!username) {
    if (state.cred && state.cred.username) {
      try {
        await invoke("clear_credential", { siteId });
        state.cred = null;
        renderCredHint();
      } catch (e) {
        /* 清除失败不影响配置保存 */
      }
    }
    return;
  }
  try {
    state.cred = await invoke("save_credential", {
      siteId,
      username,
      password: dom.pass.value || null,
      autoLogin: dom.autologin.checked,
    });
    dom.pass.value = "";
    renderCredHint();
    toast(t(credKey("toast.credSaved")));
  } catch (e) {
    toast(t("toast.credFail", { e }), true);
  }
}

async function load() {
  if (!ENABLED) return;
  try {
    const cfg = await invoke("get_config");
    state.sites = cfg.sites || [];
    state.configPath = cfg.config_path || "";
    state.version = cfg.version || "";
    state.uiTheme = cfg.ui_theme || "system";
    state.renderMode = cfg.render_mode || "auto";
    state.platform = cfg.platform || "";
    applyTheme(state.uiTheme);
    // 下拉框是在 load() 之前初始化的（那时 state.uiTheme 还是默认值），这里必须回写一次，
    // 否则读出的是 dark/light、下拉框却仍显示「跟随系统」。
    dom.theme.value = state.uiTheme;
    dom.devtools.checked = !!cfg.dev_tools;
    renderStatus();
    renderList();
    renderForm();
    renderRenderSection();
    // 已开窗口数不在配置里，单独查一次（拿不到就当作没有窗口）。
    void refreshWindowCounts();
  } catch (e) {
    toast(t("toast.loadFail", { e }), true);
  }
}

/** 重建语言下拉框：选项标签本身也是本地化的，切换语言后要跟着变。 */
function rebuildLangOptions() {
  if (!dom.lang) return;
  dom.lang.innerHTML = "";
  langOptions().forEach((opt) => {
    const o = document.createElement("option");
    o.value = opt.value;
    o.textContent = opt.label;
    dom.lang.appendChild(o);
  });
  dom.lang.value = getLang();
}

/** 渲染模式下拉框：选项标签本地化，只有 Linux 才显示这个区块。 */
function rebuildRenderOptions() {
  if (!dom.render) return;
  const cur = state.renderMode || "auto";
  dom.render.innerHTML = "";
  ["auto", "smooth", "compat"].forEach((value) => {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = t(`render.${value}`);
    dom.render.appendChild(o);
  });
  dom.render.value = cur;
}

/** 显隐「Linux 渲染」区块（按后端返回的 platform 判断）并刷新其选项。 */
function renderRenderSection() {
  if (!dom.linuxCard) return;
  dom.linuxCard.hidden = state.platform !== "linux";
  rebuildRenderOptions();
}

/** 系统标题栏的窗口标题跟着语言走（网页 document.title 与窗口标题是两回事）。 */
function syncWindowTitle() {
  if (!ENABLED || !tauri || !tauri.window) return;
  try {
    tauri.window.getCurrentWindow().setTitle(t("app.title")).catch(() => {});
  } catch (e) {
    /* 权限受限时忽略：标题保持原样即可，不影响使用 */
  }
}

/** 按当前语言重刷界面：静态文案（data-i18n）+ 所有动态区域。 */
function renderAllText() {
  applyI18n();
  syncWindowTitle();
  rebuildLangOptions();
  renderStatus();
  renderList();
  renderForm();
  renderCredHint();
  renderRenderSection();
}

/** 初始化语言切换器：填充「跟随系统 / 中文 / 英文」可选项，绑定切换事件（切换后刷新所有文案与动态区域）。 */
function initLangSelect() {
  if (!dom.lang) return;
  rebuildLangOptions();
  dom.lang.addEventListener("change", () => {
    setLang(dom.lang.value);
    renderAllText();
  });
  // 系统语言在运行中变化时（在系统设置里改显示语言、或远程会话切换）：
  // 选「跟随系统」的话界面文案要立刻跟着切。
  window.addEventListener("languagechange", () => {
    if (getLang() === "auto") renderAllText();
  });
}

/** 把主题偏好写到 <html data-theme>，CSS 据此切换变量；system 由媒体查询跟随系统。 */
function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme || "system");
}

/** 初始化主题切换器：填充「跟随系统 / 明亮 / 暗黑」，绑定切换并即时持久化。 */
function initThemeSelect() {
  if (!dom.theme) return;
  const opts = [
    { value: "system", i18n: "theme.system" },
    { value: "light", i18n: "theme.light" },
    { value: "dark", i18n: "theme.dark" },
  ];
  opts.forEach((o) => {
    const opt = document.createElement("option");
    opt.value = o.value;
    opt.setAttribute("data-i18n", o.i18n);
    opt.textContent = t(o.i18n);
    dom.theme.appendChild(opt);
  });
  dom.theme.value = state.uiTheme || "system";
  dom.theme.addEventListener("change", () => {
    const v = dom.theme.value;
    const prev = state.uiTheme;
    state.uiTheme = v;
    applyTheme(v);
    // 这里不能静默吞错：曾经因为 set_ui_theme 缺 ACL 授权被拒而没有任何提示，
    // 表现为「切主题当次看着生效、关掉窗口再开就回去了」。失败时回滚到原值。
    invoke("set_ui_theme", { theme: v }).catch((e) => {
      state.uiTheme = prev;
      applyTheme(prev);
      dom.theme.value = prev;
      toast(t("toast.setFail", { e }), true);
    });
  });
}

/**
 * 初始化 Linux 渲染模式下拉框：切换后立即落盘（后端只写配置）。
 * 渲染路径是 WebKitGTK 启动时的环境变量，所以界面提示「重启后生效」。
 */
function initRenderSelect() {
  if (!dom.render) return;
  renderRenderSection();
  dom.render.addEventListener("change", async () => {
    const mode = dom.render.value;
    try {
      state.renderMode = await invoke("set_render_mode", { mode });
      rebuildRenderOptions();
      toast(t("toast.renderSaved"));
    } catch (e) {
      rebuildRenderOptions();
      toast(t("toast.saveFail", { e }), true);
    }
  });
}

/**
 * DevTools 开关（全局，不属于某个站点）：默认关闭，只在排查页面问题时临时打开。
 * 后端只负责落盘，**对之后新建的窗口才生效** —— 已运行的窗口改不了。
 */
function initDevToolsToggle() {
  if (!dom.devtools) return;
  dom.devtools.addEventListener("change", () => {
    const enabled = dom.devtools.checked;
    invoke("set_dev_tools", { enabled }).catch((e) => {
      // 落盘失败就回滚，别留下「看着是开了、重开又回去了」的假象。
      dom.devtools.checked = !enabled;
      toast(t("toast.setFail", { e }), true);
    });
  });
}

/** 仓库地址：检查更新用（GitHub Releases API，公开仓库无需鉴权）。 */
const REPO = "albert-huan/sas-pwa-client";
const RELEASES_API = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases`;

/**
 * 比较版本号：只按「点分数字段」比，忽略 v 前缀与预发布后缀。
 * @returns {number} a > b 返回正数，相等返回 0
 */
function compareVersion(a, b) {
  const norm = (v) =>
    String(v || "")
      .replace(/^v/i, "")
      .split(/[.\-+]/)
      .map((n) => parseInt(n, 10) || 0);
  const x = norm(a);
  const y = norm(b);
  for (let i = 0; i < Math.max(x.length, y.length); i += 1) {
    const d = (x[i] || 0) - (y[i] || 0);
    if (d) return d;
  }
  return 0;
}

/**
 * 检查更新：问 GitHub 的 Releases API，有新版本就用系统浏览器打开下载页。
 *
 * 为什么不做「自动下载安装」：Tauri 的 updater 要求更新包用私钥签名、客户端内置公钥校验，
 * 需要额外维护一对签名密钥和 CI secret；发版频率不高，先用零配置的「检查 + 跳转下载页」。
 */
async function checkUpdate() {
  if (!ENABLED) return;
  if (dom.checkUpdate) dom.checkUpdate.disabled = true;
  try {
    const res = await fetch(RELEASES_API, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const info = await res.json();
    const latest = info.tag_name || "";
    const current = state.version || "0";
    if (latest && compareVersion(latest, current) > 0) {
      if (confirm(t("update.available", { latest: latest.replace(/^v/i, ""), current }))) {
        await invoke("open_url", { url: info.html_url || RELEASES_PAGE });
        toast(t("update.opened"));
      }
    } else {
      toast(t("update.latest", { current }));
    }
  } catch (e) {
    // 私有仓库的 API 会返回 404，所以文案里一并给出「手动去 Release 页看」的出路。
    toast(t("update.checkFail", { e }), true);
  } finally {
    if (dom.checkUpdate) dom.checkUpdate.disabled = false;
  }
}

if (dom.checkUpdate) {
  dom.checkUpdate.addEventListener("click", () => void checkUpdate());
}

dom.form.addEventListener("submit", async (ev) => {
  ev.preventDefault();
  await persist(true);
});
dom.save.addEventListener("click", () => persist(false));
dom.new.addEventListener("click", startNew);
dom.cancel.addEventListener("click", startNew);
dom.del.addEventListener("click", () => {
  if (state.editingIndex >= 0) removeSite(state.editingIndex);
});
dom.clearCred.addEventListener("click", async () => {
  const siteId = dom.id.value.trim();
  if (!siteId) return;
  if (!confirm(t("confirm.clearCred"))) return;
  try {
    await invoke("clear_credential", { siteId });
    state.cred = null;
    dom.user.value = "";
    dom.pass.value = "";
    dom.autologin.checked = false;
    renderCredHint();
    toast(t("toast.credCleared"));
  } catch (e) {
    toast(t("toast.clearFail", { e }), true);
  }
});
dom.close.addEventListener("click", () => {
  if (!ENABLED) return;
  core.invoke("hide_settings").catch(() => {});
});

if (!ENABLED) {
  dom.envWarn.hidden = false;
  ["btn-new", "btn-save", "btn-save-open"].forEach((id) => {
    el(id).disabled = true;
  });
}

// 设置窗重新拿到焦点时刷新「已开窗口数」：窗口是用户在站点窗那边开关的，
// 只有这时才需要重新问后端（后端查的是真实窗口表）。
window.addEventListener("focus", () => {
  void refreshWindowCounts();
});

// 启动：先应用当前语言的静态文案（含窗口标题），再初始化语言切换器与动态区域。
applyI18n();
syncWindowTitle();
initLangSelect();
initThemeSelect();
initRenderSelect();
initDevToolsToggle();
renderStatus();
renderList();
renderForm();
load();
