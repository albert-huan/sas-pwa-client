// 悬浮窗口栏的前端逻辑（独立的置顶透明窗口，页面是本地的 dock.html）。
// 位置、吸边、贴边隐藏、窗口尺寸都由 Rust（dockwin.rs）负责；这里只管渲染、交互和拖动起手。

const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const appWindow = T.window.getCurrentWindow();

const ZH = navigator.language.toLowerCase().startsWith("zh");
const TEXT = ZH ? {
  title: "窗口", empty: "暂无打开的窗口", noSite: "(未命名站点)", self: "本窗口", hidden: "已隐藏",
  newWin: "新建窗口", settings: "站点设置", look: "外观与位置", show: "显示窗口",
  pin: "钉住（常驻显示）", unpin: "取消钉住（自动隐藏）", collapse: "收起",
  hide: "隐藏到托盘（保持会话）", reload: "重新加载此窗口", close: "彻底关闭此窗口",
  rename: "重命名窗口", aliasPh: "别名，留空恢复序号", save: "保存",
  framelessOn: "启用无边框", framelessOff: "恢复系统标题栏",
  theme: "主题", opacity: "透明度", bg: "底色", accent: "强调色",
  edgeHide: "贴边时自动隐藏", reset: "重置位置", grip: "拖动", tip: "按住左侧点阵可拖动",
  themes: { midnight: "午夜蓝", graphite: "石墨", forest: "松针", dusk: "暮紫", paper: "纸白", custom: "自定义" },
} : {
  title: "Windows", empty: "No open windows", noSite: "(unnamed site)", self: "This window", hidden: "Hidden",
  newWin: "New window", settings: "Site settings", look: "Appearance & position", show: "Show window",
  pin: "Pin (keep visible)", unpin: "Unpin (auto-hide)", collapse: "Collapse",
  hide: "Hide to tray (keep session)", reload: "Reload this window", close: "Close this window",
  rename: "Rename window", aliasPh: "Alias, empty to reset", save: "Save",
  framelessOn: "Frameless mode", framelessOff: "Restore title bar",
  theme: "Theme", opacity: "Opacity", bg: "Background", accent: "Accent",
  edgeHide: "Auto-hide at screen edge", reset: "Reset position", grip: "Drag", tip: "Drag the grip to move",
  themes: { midnight: "Midnight", graphite: "Graphite", forest: "Pine", dusk: "Dusk", paper: "Paper", custom: "Custom" },
};

const THEMES = {
  midnight: { bg: "#1b2233", accent: "#7aa8ff" },
  graphite: { bg: "#26282c", accent: "#ffc857" },
  forest: { bg: "#14261f", accent: "#5fd3a0" },
  dusk: { bg: "#271f3a", accent: "#b392ff" },
  paper: { bg: "#f5f3ee", accent: "#2f6df6" },
};
const THEME_ORDER = ["midnight", "graphite", "forest", "dusk", "paper", "custom"];

const ICON = {
  plus: "<path d='M8 3.2v9.6M3.2 8h9.6'/>",
  gear: "<circle cx='8' cy='8' r='2.3'/><path d='M8 1.6v1.5M8 12.9v1.5M1.6 8h1.5M12.9 8h1.5M3.5 3.5l1.1 1.1M11.4 11.4l1.1 1.1M12.5 3.5l-1.1 1.1M4.6 11.4l-1.1 1.1'/>",
  look: "<circle cx='8' cy='8' r='5.7'/><circle cx='5.3' cy='7.2' r='.5'/><circle cx='8' cy='5' r='.5'/><circle cx='10.7' cy='7.2' r='.5'/><path d='M8 13.7c-1.3 0-1.6-1.1-1-1.9.5-.7.1-1.6-.8-1.6'/>",
  pin: "<path d='M5.6 2.2h4.8l-.7 4.1 2.1 2.2v1.1H4.2V8.5l2.1-2.2-.7-4.1zM8 9.6v4.2'/>",
  up: "<path d='M4 9.8l4-4 4 4'/>",
  down: "<path d='M4 6.2l4 4 4-4'/>",
};
const svg = (size, body) =>
  `<svg viewBox="0 0 16 16" width="${size}" height="${size}">${body}</svg>`;

const $ = (sel) => document.querySelector(sel);
const el = {
  body: document.body,
  dock: $(".dock"), handle: $(".handle"), panel: $(".panel"), grip: $(".grip"),
  chips: $(".chips"), badge: $(".brand .badge"), brandLbl: $(".brand .lbl"),
  handleN: $(".handle .n"), handleLbl: $(".handle .lbl"), dots: document.querySelectorAll(".handle .dots i"),
  newBtn: $(".tool.new"), settingsBtn: $(".tool.settings"), lookBtn: $(".tool.lookbtn"),
  pinBtn: $(".tool.pin"), collapseBtn: $(".tool.collapse"), menu: $(".menu"), look: $(".look"),
};

let pinned = false;
let open = false;
let windows = [];
let prefs = { cx: 0.5, cy: 0, edge: "", theme: "midnight", bg: "#1b2233", accent: "#7aa8ff", opacity: 0.94, edge_hide: true };
let hideTimer = 0;
let saveTimer = 0;
let selected = -1;
let lookOpen = false;
let menuOpen = false;

// ---------- 外观 ----------
function hexToRgb(h) {
  return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
}
function rgba(h, a) {
  const c = hexToRgb(h);
  return `rgba(${c[0]},${c[1]},${c[2]},${Math.max(0, Math.min(1, a))})`;
}
function mix(h1, h2, t) {
  const a = hexToRgb(h1), b = hexToRgb(h2);
  let out = "#";
  for (let i = 0; i < 3; i += 1) {
    const v = Math.round(a[i] + (b[i] - a[i]) * t);
    out += (v < 16 ? "0" : "") + v.toString(16);
  }
  return out;
}
function isLight(h) {
  const c = hexToRgb(h);
  return (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) / 255 > 0.6;
}
const isHex = (s) => typeof s === "string" && /^#[0-9a-fA-F]{6}$/.test(s);

function currentColors() {
  if (prefs.theme === "custom") return { bg: prefs.bg, accent: prefs.accent };
  return THEMES[prefs.theme] || THEMES.midnight;
}

function applyTheme() {
  const t = currentColors();
  const light = isLight(t.bg);
  const op = prefs.opacity;
  const vars = {
    "--fg": light ? "#1d2433" : "#e9eef8",
    "--fg2": light ? "#566077" : "#a3afc4",
    "--line": light ? "rgba(0,0,0,.14)" : "rgba(255,255,255,.15)",
    "--soft": light ? "rgba(0,0,0,.06)" : "rgba(255,255,255,.07)",
    "--accent": t.accent,
    "--accentSoft": rgba(t.accent, 0.18),
    "--solid": t.bg,
    "--bgGrad": `linear-gradient(180deg,${rgba(mix(t.bg, "#ffffff", light ? 0.5 : 0.07), op)},${rgba(mix(t.bg, "#000000", 0.14), op)})`,
  };
  for (const k in vars) document.documentElement.style.setProperty(k, vars[k]);
}

// ---------- 尺寸：按内容量窗口，Rust 负责定位 ----------
// 窗口四周各留 14px 给阴影。展开时如果有浮层（菜单 / 外观），临时加高。
function fit() {
  const margin = 14;
  let w, h;
  if (open) {
    w = el.dock.offsetWidth + margin * 2;
    h = el.dock.offsetHeight + margin * 2;
  } else {
    w = el.handle.offsetWidth + margin * 2;
    h = el.handle.offsetHeight + margin * 2;
  }
  if (menuOpen || lookOpen) {
    w = Math.max(w, 280);
    h = Math.max(h, 320);
  }
  invoke("dock_fit", { width: Math.ceil(w), height: Math.ceil(h), popup: menuOpen || lookOpen }).catch(() => {});
}

function setOpen(v) {
  open = !!v;
  el.body.classList.toggle("open", open);
  if (!open) {
    hideMenu();
    hideLook();
  }
  invoke("dock_set_open", { open }).catch(() => {});
  fit();
}

function reveal() {
  clearTimeout(hideTimer);
  if (!open) setOpen(true);
}

function scheduleHide() {
  if (pinned) return;
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => {
    if (pinned || lookOpen || menuOpen) return;
    setOpen(false);
  }, 600);
}

// ---------- 渲染 ----------
function chipEl(w) {
  const chip = document.createElement("div");
  chip.className = "chip" + (w.active ? " self" : "") + (w.visible ? "" : " hidden");
  chip.textContent = w.alias || String(w.index || 1);
  if (w.alias) chip.classList.add("alias");
  if (w.active) chip.style.background = w.color || "#4f8cff";
  chip.title = (w.title || w.site_name || TEXT.noSite) +
    (w.active ? " · " + TEXT.self : w.visible ? "" : " · " + TEXT.hidden);
  chip.dataset.label = w.label;
  chip.addEventListener("click", () => {
    invoke("focus_site_window", { label: w.label }).catch(() => {});
    if (!pinned) setOpen(false);
  });
  chip.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    // 阻止冒泡：document 上的 contextmenu 监听会因目标不在菜单内而立刻 hideMenu()，
    // 刚弹出的菜单会在同一事件里被关掉。
    ev.stopPropagation();
    showMenu(ev, w);
  });
  return chip;
}

function render() {
  el.chips.innerHTML = "";
  selected = -1;
  el.badge.textContent = String(windows.length);
  el.handleN.textContent = String(windows.length);
  const colors = [];
  windows.forEach((w) => { if (colors.indexOf(w.color) < 0) colors.push(w.color); });
  el.dots.forEach((d, i) => { d.style.background = colors[i] || "rgba(255,255,255,.22)"; });
  if (!windows.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = TEXT.empty;
    el.chips.appendChild(empty);
    return;
  }
  const groups = [];
  windows.forEach((w) => {
    let g = groups.find((x) => x.site_id === w.site_id);
    if (!g) {
      g = { site_id: w.site_id, site_name: w.site_name, color: w.color, items: [] };
      groups.push(g);
    }
    g.items.push(w);
  });
  groups.forEach((g) => {
    const group = document.createElement("div");
    group.className = "group";
    group.innerHTML = '<span class="gdot"></span><span class="gname"></span>';
    group.querySelector(".gdot").style.background = g.color || "#4f8cff";
    group.querySelector(".gname").textContent = g.site_name || TEXT.noSite;
    group.title = g.site_name || TEXT.noSite;
    el.chips.appendChild(group);
    g.items.forEach((w) => el.chips.appendChild(chipEl(w)));
  });
}

function applyState(snapshot) {
  if (!snapshot) return;
  const wasPinned = pinned;
  pinned = !!snapshot.pinned;
  windows = Array.isArray(snapshot.windows) ? snapshot.windows : [];
  if (snapshot.prefs && Date.now() >= editUntil) prefs = { ...prefs, ...snapshot.prefs };
  applyTheme();
  el.pinBtn.classList.toggle("on", pinned);
  el.pinBtn.title = pinned ? TEXT.unpin : TEXT.pin;
  el.body.classList.toggle("pinned", pinned);
  render();
  syncLook();
  if (pinned && !wasPinned) {
    clearTimeout(hideTimer);
    setOpen(true);
  } else if (!pinned && wasPinned) {
    scheduleHide();
  }
  fit();
}

// ---------- 拖动 ----------
// 拖动交给系统（startDragging），窗口会自然跟手，松手后 Rust 根据位置决定吸边。
// 把手要区分「点击展开」和「拖动」：移动超过 4px 才开始拖动。
function bindDrag(target, clickAction) {
  let start = null;
  target.addEventListener("pointerdown", (ev) => {
    if (ev.button !== 0) return;
    start = { x: ev.clientX, y: ev.clientY, moved: false };
  });
  target.addEventListener("pointermove", (ev) => {
    if (!start || start.moved) return;
    if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 4) return;
    start.moved = true;
    clearTimeout(hideTimer);
    hideMenu();
    hideLook();
    appWindow.startDragging().catch(() => {});
  });
  const end = () => {
    if (start && !start.moved && clickAction) clickAction();
    start = null;
  };
  target.addEventListener("pointerup", end);
  target.addEventListener("pointercancel", () => { start = null; });
}

// ---------- 偏好保存（防抖） ----------
// 本地刚改过偏好的 900ms 内，不采纳后端回传的偏好：回包里是上一次保存的值，
// 采纳它会把用户正在拖的滑块拉回去。
let editUntil = 0;
function queueSave() {
  editUntil = Date.now() + 900;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    invoke("set_dock_prefs", { prefs }).then(applyState).catch(() => {});
  }, 250);
}

// ---------- 右键菜单 ----------
function hideMenu() {
  if (!menuOpen) return;
  menuOpen = false;
  el.menu.classList.remove("show");
  fit();
}
function showMenu(ev, w) {
  hideLook();
  el.menu.innerHTML = '<div class="mt"></div>';
  el.menu.querySelector(".mt").textContent = w.title || w.site_name || TEXT.noSite;
  [
    { text: TEXT.show, cmd: "show_site_window", when: !w.visible },
    { text: TEXT.hide, cmd: "hide_site_window" },
    { text: TEXT.rename, cmd: "rename" },
    { text: w.frameless ? TEXT.framelessOff : TEXT.framelessOn, cmd: "toggle_frameless_for_label" },
    { text: TEXT.reload, cmd: "reload_site_window" },
    { text: TEXT.close, cmd: "close_site_window", danger: true },
  ].filter((a) => a.when !== false).forEach((a) => {
    const b = document.createElement("button");
    b.textContent = a.text;
    if (a.danger) b.className = "danger";
    b.addEventListener("click", (ev2) => {
      ev2.stopPropagation();
      if (a.cmd === "rename") {
        showRename(w);
        return;
      }
      hideMenu();
      invoke(a.cmd, { label: w.label }).catch(() => {});
    });
    el.menu.appendChild(b);
  });
  menuOpen = true;
  el.menu.classList.add("show");
  fit();
  el.menu.style.left = Math.min(ev.clientX, Math.max(4, el.body.clientWidth - 190)) + "px";
  el.menu.style.top = Math.min(ev.clientY + 6, Math.max(4, el.body.clientHeight - 220)) + "px";
}

// ---------- 重命名：菜单原地换成输入框，回车保存、Esc 取消 ----------
function showRename(w) {
  el.menu.innerHTML =
    '<div class="mt"></div><div class="rrow"><input type="text" class="rin" maxlength="24">' +
    '<button type="button" class="rok"></button></div>';
  el.menu.querySelector(".mt").textContent = w.title || w.site_name || TEXT.noSite;
  const inp = el.menu.querySelector(".rin");
  const okBtn = el.menu.querySelector(".rok");
  inp.value = w.alias || "";
  inp.placeholder = TEXT.aliasPh;
  okBtn.textContent = TEXT.save;
  const commit = () => {
    invoke("set_window_alias", { label: w.label, alias: inp.value })
      .then((s) => {
        applyState(s);
        hideMenu();
        if (!pinned) scheduleHide();
      })
      .catch(() => {});
  };
  okBtn.addEventListener("click", (e) => { e.stopPropagation(); commit(); });
  // 按键就地消化，别冒泡到 document 的全局键盘导航。
  inp.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Enter") { e.preventDefault(); commit(); }
    else if (e.key === "Escape") hideMenu();
  });
  inp.addEventListener("click", (e) => e.stopPropagation());
  setTimeout(() => { inp.focus(); inp.select(); }, 0);
}

// ---------- 外观面板 ----------
let ui = null;
function buildLook() {
  el.look.innerHTML =
    '<div class="sec"><h4></h4><div class="sw"></div><div class="tname"></div>' +
    '<div class="colors"><label><span class="lb"></span><input type="color" class="cbg"></label>' +
    '<label><span class="la"></span><input type="color" class="cac"></label></div></div>' +
    '<div class="sec"><h4></h4><div class="row"><input type="range" min="35" max="100" step="1" class="op">' +
    '<span class="val"></span></div></div>' +
    '<div class="sec"><label class="chk"><input type="checkbox" class="eh"><span class="le"></span></label>' +
    '<button class="link" type="button"></button></div>';
  const h4s = el.look.querySelectorAll("h4");
  h4s[0].textContent = TEXT.theme;
  h4s[1].textContent = TEXT.opacity;
  ui = {
    sw: el.look.querySelector(".sw"), tname: el.look.querySelector(".tname"),
    colors: el.look.querySelector(".colors"), op: el.look.querySelector(".op"),
    val: el.look.querySelector(".val"), bg: el.look.querySelector(".cbg"),
    ac: el.look.querySelector(".cac"), eh: el.look.querySelector(".eh"),
  };
  el.look.querySelector(".lb").textContent = TEXT.bg;
  el.look.querySelector(".la").textContent = TEXT.accent;
  el.look.querySelector(".le").textContent = TEXT.edgeHide;
  const reset = el.look.querySelector(".link");
  reset.textContent = TEXT.reset;

  THEME_ORDER.forEach((id) => {
    const b = document.createElement("button");
    b.type = "button";
    b.title = TEXT.themes[id];
    b.dataset.id = id;
    if (id === "custom") b.className = "custom";
    else {
      b.style.setProperty("--b", THEMES[id].bg);
      b.style.setProperty("--a", THEMES[id].accent);
    }
    b.addEventListener("click", () => {
      prefs.theme = id;
      if (id !== "custom") { prefs.bg = THEMES[id].bg; prefs.accent = THEMES[id].accent; }
      applyTheme(); syncLook(); queueSave();
    });
    ui.sw.appendChild(b);
  });
  const customEdit = () => {
    prefs.theme = "custom";
    prefs.bg = ui.bg.value;
    prefs.accent = ui.ac.value;
    applyTheme(); syncLook(); queueSave();
  };
  ui.bg.addEventListener("input", customEdit);
  ui.ac.addEventListener("input", customEdit);
  ui.op.addEventListener("input", () => {
    prefs.opacity = Math.max(0.35, Math.min(1, Number(ui.op.value) / 100));
    ui.val.textContent = Math.round(prefs.opacity * 100) + "%";
    applyTheme(); queueSave();
  });
  ui.eh.addEventListener("change", () => {
    prefs.edge_hide = !!ui.eh.checked;
    queueSave();
  });
  reset.addEventListener("click", () => {
    prefs.cx = 0.5; prefs.cy = 0; prefs.edge = "";
    invoke("set_dock_prefs", { prefs }).then(applyState).catch(() => {});
  });
}

function syncLook() {
  if (!ui) return;
  const cur = prefs.theme;
  Array.prototype.forEach.call(ui.sw.children, (b) => b.classList.toggle("on", b.dataset.id === cur));
  ui.tname.textContent = TEXT.themes[cur] || cur;
  ui.colors.style.display = cur === "custom" ? "flex" : "none";
  if (isHex(prefs.bg) && ui.bg.value !== prefs.bg) ui.bg.value = prefs.bg;
  if (isHex(prefs.accent) && ui.ac.value !== prefs.accent) ui.ac.value = prefs.accent;
  const pct = Math.round(prefs.opacity * 100);
  if (String(ui.op.value) !== String(pct)) ui.op.value = String(pct);
  ui.val.textContent = pct + "%";
  ui.eh.checked = !!prefs.edge_hide;
}

function showLook() {
  if (!ui) buildLook();
  hideMenu();
  clearTimeout(hideTimer);
  lookOpen = true;
  syncLook();
  el.look.classList.add("show");
  el.lookBtn.classList.add("on");
  // 面板放在胶囊正下方（窗口会被 Rust 加高到容得下它），不盖住胶囊。
  el.look.style.left = el.dock.offsetLeft + "px";
  el.look.style.top = el.dock.offsetTop + el.dock.offsetHeight + 8 + "px";
  fit();
}
function hideLook() {
  if (!lookOpen) return;
  lookOpen = false;
  el.look.classList.remove("show");
  el.lookBtn.classList.remove("on");
  fit();
  scheduleHide();
}

// ---------- 事件 ----------
bindDrag(el.grip, null);
bindDrag(el.handle, () => reveal());
el.handle.addEventListener("mouseenter", reveal);
el.body.addEventListener("mouseenter", reveal);
el.body.addEventListener("mouseleave", scheduleHide);
el.collapseBtn.addEventListener("click", () => setOpen(false));
el.pinBtn.addEventListener("click", () => {
  invoke("set_dock_pinned", { pinned: !pinned }).then(applyState).catch(() => {});
});
el.newBtn = el.newBtn || $(".tool.new");
el.newBtn.innerHTML = svg(14, ICON.plus);
el.settingsBtn.innerHTML = svg(14, ICON.gear);
el.lookBtn.innerHTML = svg(14, ICON.look);
el.pinBtn.innerHTML = svg(14, ICON.pin);
el.collapseBtn.innerHTML = svg(14, ICON.up);
el.handle.insertAdjacentHTML("beforeend", svg(10, ICON.down));
el.grip.innerHTML = svg(12, "<circle cx='5.5' cy='4' r='.9'/><circle cx='10.5' cy='4' r='.9'/><circle cx='5.5' cy='8' r='.9'/><circle cx='10.5' cy='8' r='.9'/><circle cx='5.5' cy='12' r='.9'/><circle cx='10.5' cy='12' r='.9'/>");
el.newBtn.title = TEXT.newWin;
el.settingsBtn.title = TEXT.settings;
el.lookBtn.title = TEXT.look;
el.collapseBtn.title = TEXT.collapse;
el.grip.title = TEXT.grip;
el.handleLbl = $(".handle .lbl");
el.handleLbl.textContent = TEXT.title;
$(".brand .lbl").textContent = TEXT.title;
el.handle.querySelector(".dots").insertAdjacentHTML("beforeend", "");
el.newBtn.addEventListener("click", () => invoke("new_window_for_current").catch(() => {}));
el.settingsBtn.addEventListener("click", () => invoke("show_settings").catch(() => {}));
el.lookBtn.addEventListener("click", () => (lookOpen ? hideLook() : showLook()));

document.addEventListener("click", (e) => {
  if (!el.menu.contains(e.target)) hideMenu();
  if (!el.look.contains(e.target) && !el.lookBtn.contains(e.target)) hideLook();
});
document.addEventListener("contextmenu", (e) => {
  if (!el.menu.contains(e.target)) hideMenu();
});

// 键盘导航（面板展开时）
function allChips() { return Array.prototype.slice.call(el.chips.querySelectorAll(".chip")); }
function select(delta) {
  const all = allChips();
  if (!all.length) return;
  selected = (selected + delta + all.length) % all.length;
  all.forEach((r, i) => r.classList.toggle("sel", i === selected));
  all[selected].scrollIntoView({ block: "nearest", inline: "nearest" });
}
document.addEventListener("keydown", (e) => {
  if (!open) return;
  const tag = document.activeElement && document.activeElement.tagName;
  if (tag && /^(INPUT|TEXTAREA|SELECT)$/.test(tag)) return;
  if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); select(1); }
  else if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); select(-1); }
  else if (e.key === "Enter" && selected >= 0) {
    const c = allChips()[selected];
    if (c) { e.preventDefault(); c.click(); }
  } else if (e.key === "Escape") { hideLook(); setOpen(false); }
});

// Rust 通过事件触发：Ctrl+` 切换显隐，窗口清单与偏好变化时推送快照。
T.event.listen("dock-toggle", () => setOpen(!open));
T.event.listen("windows-changed", (ev) => applyState(ev.payload));

applyTheme();
invoke("get_dock_state").then(applyState).catch(() => {});
