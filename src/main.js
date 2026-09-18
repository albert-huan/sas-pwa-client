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
  lang: el("lang-select"),
  theme: el("theme-select"),
};

let state = { sites: [], draft: null, editingIndex: -1, configPath: "", version: "", cred: null, uiTheme: "system" };

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
    dot.style.background = s.keep_awake ? "var(--ok)" : "#59606f";

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
    const url = document.createElement("div");
    url.className = "url";
    url.textContent = s.url;
    meta.append(name, url);

    const acts = document.createElement("div");
    acts.className = "acts";
    acts.append(
      mkBtn(t("list.open"), () => openSite(s.id), "primary sm"),
      mkBtn(t("list.edit"), () => startEdit(i), "sm"),
      mkBtn(s.default ? t("list.unsetDefault") : t("list.setDefault"), () => setDefault(i, !s.default), "sm"),
      mkBtn(t("list.delete"), () => removeSite(i), "sm danger")
    );

    row.append(dot, meta, acts);
    dom.list.appendChild(row);
  });
}

function mkBtn(text, onClick, cls = "") {
  const b = document.createElement("button");
  b.className = `btn ${cls}`.trim();
  b.type = "button";
  b.textContent = text;
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
    toast(t("toast.saved"));
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
  } catch (e) {
    toast(t("toast.openFail", { e }), true);
  }
}

async function removeSite(i) {
  const target = state.sites[i];
  if (!confirm(t("confirm.delete", { name: target.name || target.url }))) return;
  const sites = state.sites.filter((_, n) => n !== i);
  try {
    state.sites = await invoke("save_config", { sites });
    renderStatus();
    startNew();
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

function renderCredHint() {
  const c = state.cred;
  if (c && c.username) {
    dom.credHint.textContent = c.has_password
      ? t("cred.savedBoth", { user: c.username })
      : t("cred.savedUser", { user: c.username });
  } else {
    dom.credHint.textContent = t("cred.none");
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
    toast(t("toast.credSaved"));
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
    applyTheme(state.uiTheme);
    renderStatus();
    renderList();
    renderForm();
  } catch (e) {
    toast(t("toast.loadFail", { e }), true);
  }
}

/** 初始化语言切换器：填充「跟随系统 / 中文 / 英文」可选项，绑定切换事件（切换后刷新所有文案与动态区域）。 */
function initLangSelect() {
  if (!dom.lang) return;
  langOptions().forEach((opt) => {
    const o = document.createElement("option");
    o.value = opt.value;
    o.textContent = opt.label;
    dom.lang.appendChild(o);
  });
  dom.lang.value = getLang();
  dom.lang.addEventListener("change", () => {
    setLang(dom.lang.value);
    applyI18n();
    renderStatus();
    renderList();
    renderForm();
    renderCredHint();
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
    state.uiTheme = v;
    applyTheme(v);
    invoke("set_ui_theme", { theme: v }).catch(() => {});
  });
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

// 启动：先应用当前语言的静态文案，再初始化语言切换器与动态区域。
applyI18n();
initLangSelect();
initThemeSelect();
renderStatus();
renderList();
renderForm();
load();
