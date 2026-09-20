// 多语种支持：词典 + t() 翻译函数 + 语言记忆。无框架、无依赖。
// 用法：
//   import { t, getLang, setLang, supportedLangs, langLabel, applyI18n } from "./i18n.js";
//   - HTML 静态文案：在元素上加 data-i18n="key"（文本）/ data-i18n-ph="key"（placeholder）/
//     data-i18n-title="key"（title 属性），然后调用 applyI18n()。
//   - JS 动态文案：直接用 t("key", { var: value })。

const dict = {
  zh: {
    "app.title": "SAS 客户端 · 站点设置",
    "app.name": "SAS PWA 客户端",
    "lang.label": "语言",
    "lang.auto": "跟随系统",
    "theme.label": "主题",
    "theme.system": "跟随系统",
    "theme.light": "明亮",
    "theme.dark": "暗黑",
    "btn.close": "关闭",
    "env.warn": "未检测到 Tauri 运行环境：当前为浏览器预览，保存与打开功能不可用。",
    "sites.heading": "SAS 站点",
    "btn.new": "+ 新增",
    "editor.new": "新增 SAS 站点",
    "editor.edit": "编辑站点",
    "field.name": "名称",
    "ph.name": "例如：生产环境 / SAS Viya",
    "field.url": "SAS 站点地址 *",
    "ph.url": "https://viya.公司域名.com/",
    "opt.keep.t": "保持会话不超时（伪装 PWA）",
    "opt.keep.d": "注入 display-mode / navigator.standalone 伪装，并按下方间隔派发活动脉冲，避免 SAS 空闲超时掉线",
    "opt.frameless.t": "无边框窗口",
    "opt.frameless.d": "隐藏系统标题栏，改用自绘标题条（可拖动、最小化、⚙ 打开设置、隐藏到托盘）。站点窗口里按 F11 可随时切换该环境的有边框 / 无边框",
    "opt.default.t": "设为默认登录环境",
    "opt.default.d": "下次启动客户端时自动连接该环境；没有默认环境时，启动会先打开本设置页供选择",
    "field.user": "登录用户名",
    "ph.user": "留空表示不保存凭据",
    "field.pass": "登录密码",
    "ph.pass": "留空表示不修改已保存的密码",
    "opt.autologin.t": "自动提交登录表单（自动登录）",
    "opt.autologin.d": "出现登录页时自动填入并提交；同一会话最多自动提交一次，避免密码错误反复重试导致账号被锁",
    "field.pulse": "活动脉冲间隔（秒，0 = 关闭）",
    "btn.saveOpen": "保存并打开",
    "btn.save": "仅保存",
    "btn.clearCred": "清除已存凭据",
    "btn.delete": "删除",
    "btn.cancel": "取消",
    "status.count": "共 {n} 个站点",
    "status.config": "配置文件：{path}",
    "hint.default": "默认登录环境：{name} —— 下次启动将自动连接该环境。",
    "hint.noDefault": "未设置默认登录环境：下次启动会先打开本设置页；在右侧勾选「设为默认登录环境」即可设置。",
    "list.empty": "还没有 SAS 站点，点击右上角「新增」填写站点地址。",
    "dot.on": "已开启保活",
    "dot.off": "未开启保活",
    "list.unnamed": "(未命名)",
    "badge.default": "默认",
    "list.open": "打开",
    "list.newWindow": "新窗口",
    "list.edit": "编辑",
    "list.setDefault": "设为默认",
    "list.unsetDefault": "取消默认",
    "list.delete": "删除",
    "toast.noTauri": "当前不在 SAS 客户端中运行，无法执行该操作",
    "toast.fillUrl": "请填写访问地址",
    "toast.saved": "配置已保存",
    "toast.opening": "正在打开…",
    "toast.saveFail": "保存失败：{e}",
    "toast.openFail": "打开失败：{e}",
    "confirm.delete": "确定删除「{name}」？",
    "toast.deleted": "已删除",
    "toast.deleteFail": "删除失败：{e}",
    "toast.setDefault": "已设为默认登录环境",
    "toast.unsetDefault": "已取消默认登录环境",
    "toast.setFail": "设置失败：{e}",
    "cred.savedBoth": "已保存用户名「{user}」和密码：密码用 Windows DPAPI 加密，绑定当前 Windows 账户与本机，只有本机当前用户能解密。",
    "cred.savedUser": "已保存用户名「{user}」，未保存密码。",
    "cred.none": "未保存登录凭据。填写用户名和密码后点保存即可（密码加密存放、不回显；密码框留空表示不改动已存密码）。",
    "toast.credSaved": "登录凭据已加密保存",
    "toast.credFail": "凭据保存失败：{e}",
    "toast.loadFail": "读取配置失败：{e}",
    "confirm.clearCred": "确定清除该站点已保存的用户名和密码？",
    "toast.credCleared": "已清除登录凭据",
    "toast.clearFail": "清除失败：{e}",
    "render.heading": "Linux 渲染",
    "render.mode": "渲染模式",
    "render.auto": "自动（推荐）",
    "render.smooth": "流畅优先（保持硬件加速）",
    "render.compat": "兼容优先（关闭 DMA-BUF）",
    "render.hint": "Linux 上网页由 WebKitGTK 渲染：WebKit 默认的 DMA-BUF 硬件加速路径最快。自动模式在 WSL / NVIDIA 专有驱动下退到共享内存路径（更稳）；在没有 GPU 加速的机器上（/dev/dri 里没有 renderD* 渲染节点 —— 服务器 BMC 的 ASPEED / Matrox 等 2D 显示芯片也算）则关掉 DMA-BUF 与加速合成、走纯 CPU 合成，软渲染下这通常更稳更快。觉得卡顿可切换到「流畅优先」（保持硬件加速并强制加速合成）重启对比；出现花屏 / 闪烁 / 白屏则切回「兼容优先」。修改后需重启客户端生效。",
    "toast.renderSaved": "渲染模式已保存，重启客户端后生效",
  },
  en: {
    "app.title": "SAS Client · Site Settings",
    "app.name": "SAS PWA Client",
    "lang.label": "Language",
    "lang.auto": "Follow system",
    "theme.label": "Theme",
    "theme.system": "Follow system",
    "theme.light": "Light",
    "theme.dark": "Dark",
    "btn.close": "Close",
    "env.warn": "Tauri runtime not detected: this is a browser preview, so saving and opening are disabled.",
    "sites.heading": "SAS Sites",
    "btn.new": "+ New",
    "editor.new": "New SAS Site",
    "editor.edit": "Edit Site",
    "field.name": "Name",
    "ph.name": "e.g. Production / SAS Viya",
    "field.url": "SAS Site URL *",
    "ph.url": "https://viya.your-domain.com/",
    "opt.keep.t": "Keep session alive (PWA spoof)",
    "opt.keep.d": "Injects display-mode / navigator.standalone spoof and emits an activity pulse at the interval below to prevent SAS idle timeout.",
    "opt.frameless.t": "Frameless window",
    "opt.frameless.d": "Hides the OS title bar and uses the injected bar (drag, minimize, ⚙ settings, hide to tray). Press F11 in the site window to toggle anytime.",
    "opt.default.t": "Default sign-in environment",
    "opt.default.d": "Auto-connect this environment on next launch. With no default set, launch opens this settings page for you to choose.",
    "field.user": "Login username",
    "ph.user": "Leave blank to not save credentials",
    "field.pass": "Login password",
    "ph.pass": "Leave blank to keep the saved password unchanged",
    "opt.autologin.t": "Auto-submit login form (auto login)",
    "opt.autologin.d": "Auto-fills and submits on the login page; at most once per session to avoid account lock from repeated wrong passwords.",
    "field.pulse": "Activity pulse interval (seconds, 0 = off)",
    "btn.saveOpen": "Save and open",
    "btn.save": "Save only",
    "btn.clearCred": "Clear saved credentials",
    "btn.delete": "Delete",
    "btn.cancel": "Cancel",
    "status.count": "Total {n} sites",
    "status.config": "Config: {path}",
    "hint.default": "Default sign-in environment: {name} — will auto-connect on next launch.",
    "hint.noDefault": "No default environment set: launch opens this settings page first; check “Default sign-in environment” on the right to set one.",
    "list.empty": "No SAS sites yet. Click “+ New” (top right) to add a site address.",
    "dot.on": "Keep-alive on",
    "dot.off": "Keep-alive off",
    "list.unnamed": "(unnamed)",
    "badge.default": "Default",
    "list.open": "Open",
    "list.newWindow": "New window",
    "list.edit": "Edit",
    "list.setDefault": "Set default",
    "list.unsetDefault": "Unset default",
    "list.delete": "Delete",
    "toast.noTauri": "Not running inside the SAS client; this action is unavailable.",
    "toast.fillUrl": "Please enter the site URL",
    "toast.saved": "Configuration saved",
    "toast.opening": "Opening…",
    "toast.saveFail": "Save failed: {e}",
    "toast.openFail": "Open failed: {e}",
    "confirm.delete": "Delete “{name}”?",
    "toast.deleted": "Deleted",
    "toast.deleteFail": "Delete failed: {e}",
    "toast.setDefault": "Set as default sign-in environment",
    "toast.unsetDefault": "Default sign-in environment cleared",
    "toast.setFail": "Set failed: {e}",
    "cred.savedBoth": "Saved username “{user}” and password. The password is encrypted with Windows DPAPI, bound to the current Windows account and machine; only the current local user can decrypt it.",
    "cred.savedUser": "Saved username “{user}”; no password saved.",
    "cred.none": "No login credentials saved. Fill in the username and password and save (password is encrypted and never shown; leaving the password blank keeps the saved one unchanged).",
    "toast.credSaved": "Login credentials encrypted and saved",
    "toast.credFail": "Credential save failed: {e}",
    "toast.loadFail": "Failed to read config: {e}",
    "confirm.clearCred": "Clear the saved username and password for this site?",
    "toast.credCleared": "Login credentials cleared",
    "toast.clearFail": "Clear failed: {e}",
    "render.heading": "Linux rendering",
    "render.mode": "Rendering mode",
    "render.auto": "Automatic (recommended)",
    "render.smooth": "Performance first (keep hardware acceleration)",
    "render.compat": "Compatibility first (disable DMA-BUF)",
    "render.hint": "On Linux the page is rendered by WebKitGTK, and WebKit's default DMA-BUF path is the fastest. The automatic mode falls back to the shared-memory path on WSL / NVIDIA proprietary drivers, and on machines with no GPU acceleration (no renderD* render node under /dev/dri — the 2D BMC chips found on servers, such as ASPEED / Matrox, count as well) it additionally turns off accelerated compositing so everything is composited on the CPU, which is usually more stable and faster there. If it feels sluggish, switch to “Performance first” (keeps hardware acceleration and forces accelerated compositing) and restart; if you see flicker, artifacts or a blank window, go back to “Compatibility first”. Changes take effect after restarting the client.",
    "toast.renderSaved": "Rendering mode saved — restart the client to apply",
  },
};

const LS_KEY = "sas-client-lang";
export const SUPPORTED = ["zh", "en"];
// 语言下拉框的可选项：auto 表示「跟随系统」，它只是记忆里的**选择**，
// 不是一种语言 —— 落盘 / 存储的 auto 必须经 detectLang() 换算成 zh / en 再查词典。
const LANG_CHOICES = ["auto", "zh", "en"];

/**
 * 系统语言标签（小写，如 zh-cn / en-us）。
 * 优先用 Rust 侧注入的 `__SAS_SYS_LANG__`（Windows 上取的是系统显示语言，
 * WebView2 里的 navigator.language 未必与之一致）；注入缺失时（浏览器预览等）退回 navigator.language。
 */
function systemLangTag() {
  const clean = (v) => (typeof v === "string" ? v.trim().toLowerCase() : "");
  if (typeof window !== "undefined") {
    const injected = clean(window.__SAS_SYS_LANG__);
    if (injected) return injected;
  }
  if (typeof navigator !== "undefined") return clean(navigator.language);
  return "";
}

/**
 * auto（跟随系统）时把系统语言映射成受支持的语言：zh* → zh，其余 → en。
 * 注意：返回值只能是 "zh" / "en" —— 绝不能把「auto」这个选择本身当语言返回，
 * 否则 dict["auto"] 查不到会整体回退英文。
 */
function detectLang() {
  return systemLangTag().startsWith("zh") ? "zh" : "en";
}

// 记忆的当前选择（"auto" / "zh" / "en"）；未存时默认 "auto"（跟随系统）。
let current = (() => {
  try {
    const saved = localStorage.getItem(LS_KEY);
    if (saved && LANG_CHOICES.includes(saved)) return saved;
  } catch (e) {
    /* 忽略 */
  }
  return "auto";
})();

/** 实际生效的语言（auto 时按系统语言检测）；永远是 "zh" / "en"，不会是 "auto"。 */
export function effectiveLang() {
  return current === "zh" || current === "en" ? current : detectLang();
}

export function getLang() {
  return current;
}

export function setLang(lang) {
  if (!LANG_CHOICES.includes(lang)) return;
  current = lang;
  try {
    localStorage.setItem(LS_KEY, lang);
  } catch (e) {
    /* 忽略 */
  }
}

export function langLabel(lang) {
  const eff = effectiveLang();
  if (lang === "auto") return (dict[eff] && dict[eff]["lang.auto"]) || "Follow system";
  return lang === "zh" ? "中文" : "English";
}

/** 语言下拉框的可选项（含本地化标签）。 */
export function langOptions() {
  const eff = effectiveLang();
  return [
    { value: "auto", label: (dict[eff] && dict[eff]["lang.auto"]) || "Follow system" },
    { value: "zh", label: "中文" },
    { value: "en", label: "English" },
  ];
}

export function t(key, vars) {
  const table = dict[effectiveLang()] || dict.en;
  let s = table[key];
  if (s == null) s = dict.en[key] != null ? dict.en[key] : key;
  if (vars) {
    for (const k in vars) {
      s = s.replace(new RegExp("\\{" + k + "\\}", "g"), vars[k]);
    }
  }
  return s;
}

/** 把 data-i18n / data-i18n-ph / data-i18n-title 应用到当前文档。 */
export function applyI18n() {
  if (typeof document === "undefined") return;
  document.title = t("app.title");
  document.querySelectorAll("[data-i18n]").forEach((node) => {
    node.textContent = t(node.getAttribute("data-i18n"));
  });
  document.querySelectorAll("[data-i18n-ph]").forEach((node) => {
    node.setAttribute("placeholder", t(node.getAttribute("data-i18n-ph")));
  });
  document.querySelectorAll("[data-i18n-title]").forEach((node) => {
    node.setAttribute("title", t(node.getAttribute("data-i18n-title")));
  });
}
