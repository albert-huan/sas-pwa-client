/* 站点窗口里的「悬浮窗口栏」——由 lib.rs::render_script 注入到每个 SAS 站点窗口。
 *
 * 设计约束：
 *   - 紧凑横向胶囊：窗口按站点分组，每组一枚站点色点 + 若干窗口 chip；宽度硬上限 560px，
 *     内容再多也只在内部横向滚动，绝不能铺满屏幕；
 *   - 位置可任意拖动（按住左侧把手拖）。位置存成比例 cx/cy ∈ [0,1]，所有窗口共用一份；
 *     拖到离屏幕边缘 28px 以内会吸附到该边；
 *   - 收起态：吸附在边上且开启「贴边隐藏」时整条滑出屏幕不可见，鼠标靠近原位置的边缘热区
 *     就重新滑出；没吸附在边上时收起成一枚小把手，始终可见；可点图钉钉住；
 *   - 外观：内置 5 套主题 + 自定义底色 / 强调色，背景透明度 35%~100% 可调（文字不透明）；
 *   - 注入到远程 SAS 页面：外层容器暴露在 light DOM，布局样式全部用 !important 钉死；
 *     内部 UI 放在 closed Shadow DOM 里，样式走 adoptedStyleSheets（不受页面 CSP 限制）；
 *   - 只负责「运行中的窗口」的切换与自身外观，不碰站点配置；窗口清单与外观偏好由 Rust 的
 *     windows-changed 事件推送，不做轮询。
 *
 * 占位符（由 render_script 用 js_string 替换成带引号的 JS 字面量）：
 *   __DOCK_LABEL__  当前窗口的 label，用来标「本窗口」
 *   __DOCK_LANG__   "zh" / "en"
 */
(function () {
  // 只在主 frame 装一份：远程页面里的 iframe 不该再叠一层悬浮栏。
  if (window.top !== window) return;
  if (window.__sasDockInstalled) return;
  window.__sasDockInstalled = true;

  var SELF = __DOCK_LABEL__;
  var LANG = __DOCK_LANG__;
  var TEXT = {
    zh: {
      title: "窗口", empty: "暂无打开的窗口", noSite: "(未命名站点)",
      self: "本窗口", hidden: "已隐藏",
      newWin: "新建窗口", settings: "站点设置", look: "外观与位置",
      pin: "钉住（常驻显示）", unpin: "取消钉住（自动隐藏）", collapse: "收起",
      hide: "隐藏到托盘（保持会话）", reload: "重新加载此窗口",
      close: "彻底关闭此窗口", tip: "按住左侧把手可拖动 · Ctrl+` 切换",
      theme: "主题", opacity: "透明度", bg: "底色", accent: "强调色",
      edgeHide: "贴边时自动隐藏", reset: "重置位置", grip: "拖动",
      themes: { midnight: "午夜蓝", graphite: "石墨", forest: "松针", dusk: "暮紫", paper: "纸白", custom: "自定义" },
    },
    en: {
      title: "Windows", empty: "No open windows", noSite: "(unnamed site)",
      self: "This window", hidden: "Hidden",
      newWin: "New window", settings: "Site settings", look: "Appearance & position",
      pin: "Pin (keep visible)", unpin: "Unpin (auto-hide)", collapse: "Collapse",
      hide: "Hide to tray (keep session)", reload: "Reload this window",
      close: "Close this window", tip: "Drag the left grip to move · Ctrl+` toggle",
      theme: "Theme", opacity: "Opacity", bg: "Background", accent: "Accent",
      edgeHide: "Auto-hide at screen edge", reset: "Reset position", grip: "Drag",
      themes: { midnight: "Midnight", graphite: "Graphite", forest: "Pine", dusk: "Dusk", paper: "Paper", custom: "Custom" },
    },
  };
  var I = TEXT[LANG] || TEXT.en;
  var FONT =
    "'Segoe UI',system-ui,-apple-system,'Noto Sans CJK SC','Microsoft YaHei',sans-serif";

  // 内置主题（推荐默认：午夜蓝，深色底 + 柔和蓝强调，在浅色和深色的 SAS 页面上都不刺眼）。
  var THEMES = {
    midnight: { bg: "#1b2233", accent: "#7aa8ff" },
    graphite: { bg: "#26282c", accent: "#ffc857" },
    forest: { bg: "#14261f", accent: "#5fd3a0" },
    dusk: { bg: "#271f3a", accent: "#b392ff" },
    paper: { bg: "#f5f3ee", accent: "#2f6df6" },
  };
  var THEME_ORDER = ["midnight", "graphite", "forest", "dusk", "paper", "custom"];
  var EDGE_DIR = { top: [0, -1], bottom: [0, 1], left: [-1, 0], right: [1, 0], "": [0, -1] };
  var SNAP_PX = 28;

  function hexToRgb(h) {
    return [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
  }
  function rgba(h, a) {
    var c = hexToRgb(h);
    return "rgba(" + c[0] + "," + c[1] + "," + c[2] + "," + Math.max(0, Math.min(1, a)) + ")";
  }
  function mix(h1, h2, t) {
    var a = hexToRgb(h1), b = hexToRgb(h2), out = "#";
    for (var i = 0; i < 3; i += 1) {
      var v = Math.round(a[i] + (b[i] - a[i]) * t);
      out += (v < 16 ? "0" : "") + v.toString(16);
    }
    return out;
  }
  function isLight(h) {
    var c = hexToRgb(h);
    return (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) / 255 > 0.6;
  }
  function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }
  function isHex(s) { return typeof s === "string" && /^#[0-9a-fA-F]{6}$/.test(s); }

  // 图标尺寸写在属性上（不依赖样式表）：即使样式表被拦，只有 viewBox 的 SVG 也不会撑满容器。
  function svg(size, body) {
    return "<svg viewBox='0 0 16 16' width='" + size + "' height='" + size + "' " +
      "style='width:" + size + "px;height:" + size + "px;flex:none' " +
      "fill='none' stroke='currentColor' stroke-width='1.6' stroke-linecap='round' stroke-linejoin='round'>" +
      body + "</svg>";
  }
  var ICONS = {
    plus: svg(14, "<path d='M8 3.2v9.6M3.2 8h9.6'/>"),
    gear: svg(14, "<circle cx='8' cy='8' r='2.3'/><path d='M8 1.6v1.5M8 12.9v1.5M1.6 8h1.5M12.9 8h1.5M3.5 3.5l1.1 1.1M11.4 11.4l1.1 1.1M12.5 3.5l-1.1 1.1M4.6 11.4l-1.1 1.1'/>"),
    pin: svg(14, "<path d='M5.6 2.2h4.8l-.7 4.1 2.1 2.2v1.1H4.2V8.5l2.1-2.2-.7-4.1zM8 9.6v4.2'/>"),
    chev: svg(10, "<path d='M4 6.2l4 4 4-4'/>"),
    up: svg(14, "<path d='M4 9.8l4-4 4 4'/>"),
    look: svg(14, "<circle cx='8' cy='8' r='5.7'/><circle cx='5.3' cy='7.2' r='.5'/><circle cx='8' cy='5' r='.5'/><circle cx='10.7' cy='7.2' r='.5'/><path d='M8 13.7c-1.3 0-1.6-1.1-1-1.9.5-.7.1-1.6-.8-1.6'/>"),
  };

  function invoke(cmd, args) {
    var c = window.__TAURI__ && window.__TAURI__.core;
    if (!c || typeof c.invoke !== "function") return Promise.reject(new Error("no tauri"));
    return c.invoke(cmd, args);
  }

  var pinned = false;
  var open = false;
  var windows = [];
  var hideTimer = 0;
  var selected = -1;
  var host;
  var drag = null;
  // 与 Rust 的 config::DockPrefs 同构；初始值 = 默认值，随后被 get_dock_state 的快照覆盖。
  var prefs = {
    cx: 0.5, cy: 0, edge: "", theme: "midnight",
    bg: "#1b2233", accent: "#7aa8ff", opacity: 0.94, edge_hide: true,
  };

  function boot() {
    host = document.createElement("div");
    host.id = "sas-dock-host";
    // host 在远程页面的 light DOM 里：逐条用 !important 钉死布局。
    // 它铺满视口但 pointer-events:none（点击全部穿透），可见元素在 Shadow DOM 里各自开启事件。
    // 用 left/top/right/bottom 而不是 100vw：fixed 的包含块不含滚动条，100vw 会多出一条滚动条宽度。
    var set = function (k, v) { host.style.setProperty(k, v, "important"); };
    set("position", "fixed");
    set("left", "0");
    set("top", "0");
    set("right", "0");
    set("bottom", "0");
    set("width", "auto");
    set("height", "auto");
    set("margin", "0");
    set("padding", "0");
    set("border", "0");
    set("background", "transparent");
    set("box-sizing", "border-box");
    set("overflow", "visible");
    set("z-index", "2147483647");
    set("pointer-events", "none");
    set("font-family", FONT);
    (document.documentElement || document.body).appendChild(host);

    var root = host.attachShadow({ mode: "closed" });
    var CSS = [
      ":host{all:initial}",
      "*{box-sizing:border-box}",
      "@keyframes sasPop{from{opacity:0;transform:translateY(-4px)}to{opacity:1;transform:none}}",
      /* 外层：绝对定位的容器，位置由脚本按比例算出；handle 与 panel 叠在同一格里，容器尺寸 = 展开态尺寸 */
      ".dock{position:absolute;left:0;top:0;width:max-content;max-width:min(92vw,560px);display:grid;",
      "pointer-events:none;color:var(--fg);font:12.5px/1.4 " + FONT + ";",
      "transition:transform .28s cubic-bezier(.2,.8,.2,1)}",
      ".dock.dragging{transition:none;user-select:none;-webkit-user-select:none}",
      ".dock>.handle,.dock>.panel{grid-area:1/1}",
      /* 贴边时的隐形热区：只在「贴边隐藏」开启时存在，鼠标靠近就唤出 */
      ".hot{position:absolute;pointer-events:auto;display:none}",
      ".hot.on{display:block}",
      /* 收起态小把手 */
      ".handle{justify-self:center;align-self:center;display:flex;align-items:center;gap:7px;",
      "height:26px;padding:0 10px;border-radius:999px;pointer-events:auto;cursor:pointer;",
      "background:var(--bgGrad);border:1px solid var(--line);color:var(--fg);",
      "box-shadow:0 6px 18px rgba(0,0,0,.3),inset 0 1px 0 var(--soft);",
      "backdrop-filter:blur(14px) saturate(150%);",
      "font:600 11px/1 " + FONT + ";letter-spacing:.3px;white-space:nowrap;",
      "transition:opacity .18s,transform .24s cubic-bezier(.2,.8,.2,1)}",
      ".dock[data-edge=top] .handle{align-self:start}.dock[data-edge=bottom] .handle{align-self:end}",
      ".dock[data-edge=left] .handle{justify-self:start}.dock[data-edge=right] .handle{justify-self:end}",
      ".handle:hover{border-color:var(--accent)}",
      ".handle .dots{display:flex;gap:3px}",
      ".handle .dots i{width:6px;height:6px;border-radius:50%;display:block}",
      ".handle .n{color:var(--accent);font-variant-numeric:tabular-nums}",
      ".handle .lbl{color:var(--fg2)}",
      ".dock.open .handle{opacity:0;transform:scale(.92);pointer-events:none}",
      /* 展开态：紧凑横向胶囊，宽度由内容决定，硬上限 560px */
      ".panel{display:flex;align-items:center;gap:9px;width:max-content;max-width:min(92vw,560px);",
      "padding:6px 7px 6px 3px;border-radius:15px;color:var(--fg);",
      "background:var(--bgGrad);border:1px solid var(--line);",
      "box-shadow:0 14px 36px rgba(0,0,0,.34),0 2px 8px rgba(0,0,0,.2),inset 0 1px 0 var(--soft);",
      "backdrop-filter:blur(18px) saturate(150%);",
      "opacity:0;transform:scale(.97);pointer-events:none;",
      "transition:transform .24s cubic-bezier(.2,.8,.2,1),opacity .2s,border-color .15s}",
      ".dock.open .panel{opacity:1;transform:none;pointer-events:auto}",
      ".dock.snap .panel{border-color:var(--accent)}",
      ".grip{width:16px;align-self:stretch;flex:none;display:flex;align-items:center;justify-content:center;",
      "cursor:grab;color:var(--fg2);border-radius:9px;touch-action:none}",
      ".grip:hover{background:var(--soft);color:var(--fg)}",
      ".dock.dragging .grip{cursor:grabbing}",
      ".brand{display:flex;align-items:center;gap:6px;flex:none}",
      ".brand .badge{display:inline-flex;align-items:center;justify-content:center;min-width:23px;height:22px;",
      "padding:0 6px;border-radius:7px;background:var(--accentSoft);color:var(--accent);",
      "font-size:11px;font-weight:700;font-variant-numeric:tabular-nums}",
      ".brand .lbl{font-size:11px;color:var(--fg2);font-weight:600}",
      ".chips{display:flex;align-items:center;gap:8px;flex:0 1 auto;min-width:0;",
      "max-width:min(46vw,300px);overflow-x:auto;padding:2px 1px 4px;scrollbar-width:none}",
      ".chips::-webkit-scrollbar{display:none}",
      ".empty{color:var(--fg2);font-size:11.5px;padding:3px 8px;white-space:nowrap}",
      ".group{display:flex;align-items:center;gap:4px;flex:none;padding-left:7px;border-left:1px solid var(--line)}",
      ".group:first-child{padding-left:0;border-left:0}",
      ".gdot{width:8px;height:8px;border-radius:50%;flex:none;box-shadow:0 0 0 3px var(--soft)}",
      ".gname{font-size:10.5px;color:var(--fg2);max-width:68px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}",
      ".chip{display:flex;align-items:center;justify-content:center;min-width:23px;height:23px;padding:0 6px;",
      "border-radius:7px;border:1px solid var(--line);background:var(--soft);",
      "color:var(--fg);font-size:11px;font-weight:700;font-variant-numeric:tabular-nums;cursor:pointer;user-select:none;flex:none;",
      "transition:transform .15s,background .15s,border-color .15s,box-shadow .15s}",
      ".chip:hover{transform:translateY(-1px);border-color:var(--accent)}",
      ".chip.self{color:#fff;border-color:transparent;box-shadow:0 3px 10px rgba(0,0,0,.3),inset 0 0 0 1px rgba(255,255,255,.22)}",
      ".chip.hidden{border-style:dashed;color:var(--fg2);background:transparent}",
      ".chip.sel{outline:2px solid var(--accent);outline-offset:2px}",
      ".tools{display:flex;align-items:center;gap:2px;flex:none;padding-left:7px;border-left:1px solid var(--line)}",
      ".tool{width:26px;height:26px;display:flex;align-items:center;justify-content:center;border-radius:8px;",
      "border:1px solid transparent;background:transparent;color:var(--fg2);cursor:pointer;padding:0;",
      "transition:background .15s,color .15s,border-color .15s}",
      ".tool:hover{background:var(--soft);color:var(--fg)}",
      ".tool.on{color:var(--accent);background:var(--accentSoft);border-color:var(--accent)}",
      /* 浮层（右键菜单 / 外观面板）统一用实心底，不随透明度变，保证可读 */
      ".menu,.look{position:absolute;display:none;pointer-events:auto;border-radius:11px;color:var(--fg);",
      "background:var(--solid);border:1px solid var(--line);",
      "box-shadow:0 14px 34px rgba(0,0,0,.4),inset 0 1px 0 var(--soft)}",
      ".menu{min-width:176px;padding:5px}",
      ".menu.show,.look.show{display:block;animation:sasPop .16s ease}",
      ".menu .mt{font-size:10px;color:var(--fg2);padding:4px 9px 6px;margin-bottom:4px;",
      "border-bottom:1px solid var(--line);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}",
      ".menu button{display:block;width:100%;padding:7px 9px;border:0;border-radius:7px;",
      "background:transparent;color:var(--fg);font:11.5px/1.3 " + FONT + ";text-align:left;cursor:pointer}",
      ".menu button:hover{background:var(--soft)}",
      ".menu button.danger{color:#ff7a70}",
      ".look{width:236px;padding:10px 11px 11px}",
      ".look h4{margin:0 0 6px;font:600 10.5px/1.2 " + FONT + ";color:var(--fg2);letter-spacing:.4px}",
      ".look .sec{margin-bottom:10px}.look .sec:last-child{margin-bottom:0}",
      ".sw{display:grid;grid-template-columns:repeat(6,1fr);gap:6px}",
      ".sw button{height:30px;padding:0;border-radius:8px;border:2px solid transparent;cursor:pointer;",
      "position:relative;overflow:hidden;background:var(--b);box-shadow:inset 0 0 0 1px var(--line)}",
      ".sw button::after{content:'';position:absolute;right:3px;bottom:3px;width:9px;height:9px;border-radius:50%;background:var(--a)}",
      ".sw button.custom{background:conic-gradient(#ff6b6b,#ffd166,#5fd3a0,#7aa8ff,#b392ff,#ff6b6b)}",
      ".sw button.custom::after{display:none}",
      ".sw button.on{border-color:var(--accent)}",
      ".sw button:hover{transform:translateY(-1px)}",
      ".tname{margin-top:5px;font-size:11px;color:var(--fg2)}",
      ".row{display:flex;align-items:center;gap:8px}",
      ".row input[type=range]{flex:1;min-width:0;accent-color:var(--accent);margin:0}",
      ".row .val{width:34px;text-align:right;font-size:11px;font-variant-numeric:tabular-nums;color:var(--fg2)}",
      ".colors{display:flex;gap:10px;margin-top:8px}",
      ".colors label{display:flex;align-items:center;gap:6px;font-size:11px;color:var(--fg2);flex:1}",
      ".colors input[type=color]{width:34px;height:24px;padding:0;border:1px solid var(--line);border-radius:6px;background:transparent;cursor:pointer}",
      ".chk{display:flex;align-items:center;gap:7px;font-size:11.5px;color:var(--fg);cursor:pointer}",
      ".chk input{accent-color:var(--accent);margin:0}",
      ".link{margin-top:9px;padding:0;border:0;background:none;color:var(--accent);font:11.5px/1 " + FONT + ";cursor:pointer}",
      ".link:hover{text-decoration:underline}",
      /* 低透明度保护：文字颜色按主题底色选（深底浅字 / 浅底深字），但透明后实际衬在页面上，
         页面底色和字色相近就会看不清。给文字和图标加一圈主题底色的光晕，不论页面是深是浅都有对比；
         不透明度高时光晕与底色融为一体，看不出来。浮层本身是实心底，不需要。 */
      ".panel,.handle{text-shadow:0 0 2px var(--solid),0 0 4px var(--solid),0 0 7px var(--solid)}",
      ".panel svg,.handle svg{filter:drop-shadow(0 0 1.5px var(--solid)) drop-shadow(0 0 4px var(--solid))}",
      ".chip.self{text-shadow:none}",
      ".menu,.look{text-shadow:none}",
      ".menu svg,.look svg{filter:none}",
    ].join("");
    // 样式优先走 constructable stylesheet：不受页面 CSP 的 style-src 限制；不支持时退回 <style>。
    try {
      if (typeof CSSStyleSheet === "function" && "adoptedStyleSheets" in root) {
        var sheet = new CSSStyleSheet();
        sheet.replaceSync(CSS);
        root.adoptedStyleSheets = [sheet];
      } else { throw new Error("no adoptedStyleSheets"); }
    } catch (e) {
      var st = document.createElement("style");
      st.textContent = CSS;
      root.appendChild(st);
    }
    var body = document.createElement("div");
    body.innerHTML =
      '<div class="hot"></div>' +
      '<div class="dock" data-edge="top">' +
      '  <div class="handle"><span class="dots"></span><span class="n"></span><span class="lbl"></span></div>' +
      '  <div class="panel">' +
      '    <div class="grip"></div>' +
      '    <div class="brand"><span class="badge"></span><span class="lbl"></span></div>' +
      '    <div class="chips"></div>' +
      '    <div class="tools">' +
      '      <button class="tool new"></button><button class="tool settings"></button>' +
      '      <button class="tool lookbtn"></button><button class="tool pin"></button>' +
      '      <button class="tool collapse"></button>' +
      "    </div>" +
      "  </div>" +
      "</div>" +
      '<div class="menu"></div>' +
      '<div class="look"></div>';
    while (body.firstChild) root.appendChild(body.firstChild);

    var hot = root.querySelector(".hot");
    var dock = root.querySelector(".dock");
    var handle = root.querySelector(".handle");
    var panel = root.querySelector(".panel");
    var grip = root.querySelector(".grip");
    var chips = root.querySelector(".chips");
    var menu = root.querySelector(".menu");
    var look = root.querySelector(".look");
    var pinBtn = root.querySelector(".pin");
    var collapseBtn = root.querySelector(".collapse");
    var newBtn = root.querySelector(".new");
    var settingsBtn = root.querySelector(".settings");
    var lookBtn = root.querySelector(".lookbtn");
    var brandNum = root.querySelector(".brand .badge");
    var handleNum = root.querySelector(".handle .n");

    root.querySelector(".brand .lbl").textContent = I.title;
    handle.querySelector(".lbl").textContent = I.title;
    handle.querySelector(".dots").insertAdjacentHTML("beforeend", "<i></i><i></i><i></i>");
    handle.insertAdjacentHTML("beforeend", ICONS.chev);
    grip.innerHTML = svg(12, "<circle cx='5.5' cy='4' r='.9'/><circle cx='10.5' cy='4' r='.9'/>" +
      "<circle cx='5.5' cy='8' r='.9'/><circle cx='10.5' cy='8' r='.9'/>" +
      "<circle cx='5.5' cy='12' r='.9'/><circle cx='10.5' cy='12' r='.9'/>");
    newBtn.innerHTML = ICONS.plus;
    settingsBtn.innerHTML = ICONS.gear;
    lookBtn.innerHTML = ICONS.look;
    pinBtn.innerHTML = ICONS.pin;
    collapseBtn.innerHTML = ICONS.up;
    newBtn.title = I.newWin;
    settingsBtn.title = I.settings;
    lookBtn.title = I.look;
    collapseBtn.title = I.collapse;
    grip.title = I.grip;
    panel.title = I.tip;

    // 无边框模式下自绘标题条占顶部 28px：贴顶的位置要让开它。
    var barInset = 0;
    var M = 6; // 离屏幕边缘的留白（吸附在边上时用 4）
    var lookOpen = false;
    var saveTimer = 0, editUntil = 0, justDragged = 0;

    // ---- 外观：主题 → CSS 变量（设在 host 上，经继承进入 Shadow DOM）----
    function applyTheme() {
      var t = prefs.theme === "custom" ? { bg: prefs.bg, accent: prefs.accent } : (THEMES[prefs.theme] || THEMES.midnight);
      var light = isLight(t.bg);
      var op = prefs.opacity;
      var vars = {
        "--fg": light ? "#1d2433" : "#e9eef8",
        "--fg2": light ? "#566077" : "#a3afc4",
        "--line": light ? "rgba(0,0,0,.14)" : "rgba(255,255,255,.15)",
        "--soft": light ? "rgba(0,0,0,.06)" : "rgba(255,255,255,.07)",
        "--accent": t.accent,
        "--accentSoft": rgba(t.accent, 0.18),
        "--solid": t.bg,
        "--bgGrad": "linear-gradient(180deg," + rgba(mix(t.bg, "#ffffff", light ? 0.5 : 0.07), op) + "," +
          rgba(mix(t.bg, "#000000", 0.14), op) + ")",
      };
      for (var k in vars) host.style.setProperty(k, vars[k]);
    }

    // ---- 位置：比例 → 像素 ----
    function range(edge) {
      var m = edge ? 4 : M;
      var W = host.clientWidth, H = host.clientHeight;
      var dw = dock.offsetWidth, dh = dock.offsetHeight;
      var minX = m, minY = m + barInset;
      return { minX: minX, maxX: Math.max(minX, W - dw - m), minY: minY, maxY: Math.max(minY, H - dh - m),
        W: W, H: H, dw: dw, dh: dh };
    }
    function hiddenAtEdge() {
      return !open && !pinned && !drag && !!prefs.edge && prefs.edge_hide;
    }
    function layout() {
      var r = range(prefs.edge);
      var x = r.minX + prefs.cx * (r.maxX - r.minX);
      var y = r.minY + prefs.cy * (r.maxY - r.minY);
      dock.style.left = x + "px";
      dock.style.top = y + "px";
      dock.setAttribute("data-edge", prefs.edge);
      var tx = 0, ty = 0, away = hiddenAtEdge();
      if (away) {
        if (prefs.edge === "top") ty = -(y + r.dh + 30);
        else if (prefs.edge === "bottom") ty = r.H - y + 30;
        else if (prefs.edge === "left") tx = -(x + r.dw + 30);
        else if (prefs.edge === "right") tx = r.W - x + 30;
      }
      dock.style.transform = (tx || ty) ? "translate(" + tx + "px," + ty + "px)" : "";
      // 贴边隐藏时，在原位置的屏幕边缘放一条隐形热区，鼠标靠近就唤出。
      hot.classList.toggle("on", away);
      if (away) {
        var thick = 8, hs = hot.style;
        if (prefs.edge === "top" || prefs.edge === "bottom") {
          var len = Math.max(Math.min(r.dw, 320), 160);
          hs.width = len + "px"; hs.height = thick + "px";
          hs.left = clamp(x + r.dw / 2 - len / 2, 0, Math.max(0, r.W - len)) + "px";
          // 无边框模式下顶部 28px 被自绘标题条（同样是最高层级）盖住，热区要躲到它下面，否则够不着。
          hs.top = (prefs.edge === "top" ? barInset : r.H - thick) + "px";
        } else {
          var h2 = 140;
          hs.width = thick + "px"; hs.height = h2 + "px";
          hs.top = clamp(y + r.dh / 2 - h2 / 2, 0, Math.max(0, r.H - h2)) + "px";
          hs.left = (prefs.edge === "left" ? 0 : r.W - thick) + "px";
        }
      }
    }
    function nearEdge(l, t, r) {
      var d = { top: t - barInset, left: l, right: r.W - (l + r.dw), bottom: r.H - (t + r.dh) };
      var best = "", bv = SNAP_PX + 1;
      for (var k in d) { if (d[k] < bv) { bv = d[k]; best = k; } }
      return best;
    }

    function hideMenu() { menu.classList.remove("show"); }
    function hideLook() {
      if (!lookOpen) return;
      lookOpen = false;
      look.classList.remove("show");
      lookBtn.classList.remove("on");
      scheduleHide();
    }
    function setOpen(v) {
      open = !!v;
      dock.classList.toggle("open", open);
      if (!open) { hideMenu(); hideLook(); }
      layout();
    }
    function scheduleHide() {
      if (pinned) return;
      clearTimeout(hideTimer);
      hideTimer = setTimeout(function () {
        if (pinned || lookOpen || drag) return;
        // 鼠标还停在面板上（例如刚拖完）就别收，等 mouseleave 再来。
        try { if (panel.matches(":hover")) return; } catch (e) {}
        setOpen(false);
      }, 600);
    }
    function reveal() {
      clearTimeout(hideTimer);
      setOpen(true);
    }
    // ---- 偏好保存：防抖后一次写入，Rust 清洗落盘并广播给其他窗口 ----
    function queueSave() {
      editUntil = Date.now() + 900; // 这段时间内忽略自己写入引发的回声，避免滑块来回跳
      clearTimeout(saveTimer);
      saveTimer = setTimeout(function () {
        invoke("set_dock_prefs", { prefs: prefs }).catch(function () {});
      }, 250);
    }

    // ---- 拖动：按住左侧把手（或收起态的小把手）拖，松手时吸附到近边 ----
    function startDrag(ev, el) {
      if (ev.button !== 0 && ev.pointerType === "mouse") return;
      drag = { id: ev.pointerId, el: el, sx: ev.clientX, sy: ev.clientY,
        left: dock.offsetLeft, top: dock.offsetTop, moved: false };
      try { el.setPointerCapture(ev.pointerId); } catch (e) {}
    }
    function moveDrag(ev) {
      if (!drag || ev.pointerId !== drag.id) return;
      var dx = ev.clientX - drag.sx, dy = ev.clientY - drag.sy;
      if (!drag.moved) {
        if (Math.abs(dx) + Math.abs(dy) < 4) return;
        drag.moved = true;
        clearTimeout(hideTimer);
        hideLook();
        dock.classList.add("dragging");
        dock.style.transform = "";
        setOpen(true); // 拖动时展开，能看到完整胶囊落在哪
      }
      var r = range("");
      var l = clamp(drag.left + dx, 0, Math.max(0, r.W - r.dw));
      var t = clamp(drag.top + dy, 0, Math.max(0, r.H - r.dh));
      dock.style.left = l + "px";
      dock.style.top = t + "px";
      dock.classList.toggle("snap", !!nearEdge(l, t, r));
      ev.preventDefault();
    }
    function endDrag(ev) {
      if (!drag || (ev && ev.pointerId !== drag.id)) return;
      var d = drag;
      drag = null;
      try { d.el.releasePointerCapture(d.id); } catch (e) {}
      dock.classList.remove("snap");
      dock.classList.remove("dragging");
      if (!d.moved) return;
      justDragged = Date.now();
      var l = dock.offsetLeft, t = dock.offsetTop;
      var edge = nearEdge(l, t, range(""));
      var r = range(edge);
      prefs.edge = edge;
      prefs.cx = r.maxX > r.minX ? clamp((l - r.minX) / (r.maxX - r.minX), 0, 1) : 0.5;
      prefs.cy = r.maxY > r.minY ? clamp((t - r.minY) / (r.maxY - r.minY), 0, 1) : 0.5;
      if (edge === "top") prefs.cy = 0;
      else if (edge === "bottom") prefs.cy = 1;
      else if (edge === "left") prefs.cx = 0;
      else if (edge === "right") prefs.cx = 1;
      layout();
      queueSave();
      scheduleHide();
    }
    [grip, handle].forEach(function (el) {
      el.addEventListener("pointerdown", function (ev) { startDrag(ev, el); });
      el.addEventListener("pointermove", moveDrag);
      el.addEventListener("pointerup", endDrag);
      el.addEventListener("pointercancel", endDrag);
    });

    // ---- 外观面板：主题 / 透明度 / 自定义色 / 贴边隐藏 ----
    var sw, tname, colorsBox, opIn, opVal, bgIn, acIn, ehIn;
    function buildLook() {
      look.innerHTML =
        '<div class="sec"><h4></h4><div class="sw"></div><div class="tname"></div>' +
        '<div class="colors"><label><span class="lb"></span><input type="color" class="cbg"></label>' +
        '<label><span class="la"></span><input type="color" class="cac"></label></div></div>' +
        '<div class="sec"><h4></h4><div class="row"><input type="range" min="35" max="100" step="1" class="op">' +
        '<span class="val"></span></div></div>' +
        '<div class="sec"><label class="chk"><input type="checkbox" class="eh"><span class="le"></span></label>' +
        '<button class="link" type="button"></button></div>';
      var h4s = look.querySelectorAll("h4");
      h4s[0].textContent = I.theme;
      h4s[1].textContent = I.opacity;
      sw = look.querySelector(".sw");
      tname = look.querySelector(".tname");
      colorsBox = look.querySelector(".colors");
      opIn = look.querySelector(".op");
      opVal = look.querySelector(".val");
      bgIn = look.querySelector(".cbg");
      acIn = look.querySelector(".cac");
      ehIn = look.querySelector(".eh");
      look.querySelector(".lb").textContent = I.bg;
      look.querySelector(".la").textContent = I.accent;
      look.querySelector(".le").textContent = I.edgeHide;
      var resetBtn = look.querySelector(".link");
      resetBtn.textContent = I.reset;

      THEME_ORDER.forEach(function (id) {
        var b = document.createElement("button");
        b.type = "button";
        b.title = I.themes[id];
        b.dataset.id = id;
        if (id === "custom") b.className = "custom";
        else {
          b.style.setProperty("--b", THEMES[id].bg);
          b.style.setProperty("--a", THEMES[id].accent);
        }
        b.addEventListener("click", function () {
          prefs.theme = id;
          if (id !== "custom") { prefs.bg = THEMES[id].bg; prefs.accent = THEMES[id].accent; }
          applyTheme(); syncLook(); queueSave();
        });
        sw.appendChild(b);
      });
      function customEdit() {
        prefs.theme = "custom";
        prefs.bg = bgIn.value;
        prefs.accent = acIn.value;
        applyTheme(); syncLook(); queueSave();
      }
      bgIn.addEventListener("input", customEdit);
      acIn.addEventListener("input", customEdit);
      opIn.addEventListener("input", function () {
        prefs.opacity = clamp(Number(opIn.value) / 100, 0.35, 1);
        opVal.textContent = Math.round(prefs.opacity * 100) + "%";
        applyTheme(); queueSave();
      });
      ehIn.addEventListener("change", function () {
        prefs.edge_hide = !!ehIn.checked;
        layout(); queueSave();
      });
      resetBtn.addEventListener("click", function () {
        prefs.cx = 0.5; prefs.cy = 0; prefs.edge = "";
        layout(); queueSave();
      });
    }
    function syncLook() {
      if (!sw) return;
      var cur = THEMES[prefs.theme] ? prefs.theme : (prefs.theme === "custom" ? "custom" : "midnight");
      Array.prototype.forEach.call(sw.children, function (b) {
        b.classList.toggle("on", b.dataset.id === cur);
      });
      tname.textContent = I.themes[cur];
      colorsBox.style.display = cur === "custom" ? "flex" : "none";
      if (isHex(prefs.bg) && bgIn.value !== prefs.bg) bgIn.value = prefs.bg;
      if (isHex(prefs.accent) && acIn.value !== prefs.accent) acIn.value = prefs.accent;
      var pct = Math.round(prefs.opacity * 100);
      if (String(opIn.value) !== String(pct)) opIn.value = String(pct);
      opVal.textContent = pct + "%";
      ehIn.checked = !!prefs.edge_hide;
    }
    function showLook() {
      if (!sw) buildLook();
      hideMenu();
      clearTimeout(hideTimer);
      lookOpen = true;
      syncLook();
      look.classList.add("show");
      lookBtn.classList.add("on");
      var r = range(prefs.edge), dr = dock.getBoundingClientRect(), br = lookBtn.getBoundingClientRect();
      var lw = look.offsetWidth, lh = look.offsetHeight;
      look.style.left = clamp(br.left + br.width / 2 - lw / 2, 6, Math.max(6, r.W - lw - 6)) + "px";
      var below = dr.top + dr.height / 2 < r.H / 2;
      look.style.top = clamp(below ? dr.bottom + 8 : dr.top - lh - 8, 6, Math.max(6, r.H - lh - 6)) + "px";
    }
    lookBtn.addEventListener("click", function () { lookOpen ? hideLook() : showLook(); });

    hot.addEventListener("mouseenter", function () { reveal(); scheduleHide(); });
    handle.addEventListener("mouseenter", function () { reveal(); scheduleHide(); });
    handle.addEventListener("click", function () {
      if (Date.now() - justDragged < 300) return;
      reveal();
    });
    panel.addEventListener("mouseenter", function () { clearTimeout(hideTimer); });
    panel.addEventListener("mouseleave", scheduleHide);
    collapseBtn.addEventListener("click", function () { setOpen(false); });
    window.addEventListener("resize", layout);
    pinBtn.addEventListener("click", function () {
      invoke("set_dock_pinned", { pinned: !pinned }).then(window.applyDockState).catch(function () {});
    });
    newBtn.addEventListener("click", function () {
      invoke("new_window_for_current").catch(function () {});
    });
    settingsBtn.addEventListener("click", function () {
      invoke("show_settings").catch(function () {});
    });

    // ---- 右键菜单：窗口级动作（隐藏 / 重载 / 彻底关闭）----
    function showMenu(ev, w) {
      hideLook();
      menu.innerHTML = '<div class="mt"></div>';
      menu.querySelector(".mt").textContent = w.title || w.site_name || I.noSite;
      [
        { text: I.hide, cmd: "hide_site_window" },
        { text: I.reload, cmd: "reload_site_window" },
        { text: I.close, cmd: "close_site_window", danger: true },
      ].forEach(function (a) {
        var b = document.createElement("button");
        b.textContent = a.text;
        if (a.danger) b.className = "danger";
        b.addEventListener("click", function (ev2) {
          ev2.stopPropagation();
          hideMenu();
          invoke(a.cmd, { label: w.label }).catch(function () {});
        });
        menu.appendChild(b);
      });
      menu.classList.add("show");
      var W = host.clientWidth, H = host.clientHeight;
      var mw = menu.offsetWidth, mh = menu.offsetHeight;
      menu.style.left = clamp(ev.clientX, 4, Math.max(4, W - mw - 4)) + "px";
      menu.style.top = clamp(ev.clientY + 6, 4, Math.max(4, H - mh - 4)) + "px";
    }
    // closed Shadow DOM 里的点击在 document 上会被重定向成 host：target === host 说明点在悬浮栏自己身上。
    document.addEventListener("click", function (e) {
      if (e.target === host) return;
      hideMenu();
      hideLook();
    }, true);
    document.addEventListener("contextmenu", function (e) {
      // 页面自己的右键菜单照常，只关掉我们这一层。
      if (e.target !== host) { hideMenu(); hideLook(); }
    }, true);

    // ---- 列表渲染：按站点分组，同站点的多个窗口收在一组里 ----
    function chipEl(w) {
      var chip = document.createElement("div");
      chip.className = "chip" + (w.label === SELF ? " self" : "") + (w.visible ? "" : " hidden");
      chip.textContent = String(w.index || 1);
      if (w.label === SELF) chip.style.background = w.color || "#4f8cff";
      chip.title = (w.title || w.site_name || I.noSite) +
        (w.label === SELF ? " · " + I.self : w.visible ? "" : " · " + I.hidden);
      chip.dataset.label = w.label;
      chip.addEventListener("click", function () {
        invoke("focus_site_window", { label: w.label }).catch(function () {});
        if (!pinned) setOpen(false);
      });
      chip.addEventListener("contextmenu", function (ev) {
        ev.preventDefault();
        ev.stopPropagation();
        showMenu(ev, w);
      });
      return chip;
    }

    function render() {
      chips.innerHTML = "";
      selected = -1;
      brandNum.textContent = String(windows.length);
      handleNum.textContent = String(windows.length);
      // 把手上的三个色点取前三个站点色，让收起态也有一眼可辨的分组信息。
      var dots = handle.querySelectorAll(".dots i");
      var colors = [];
      windows.forEach(function (w) {
        if (colors.indexOf(w.color) < 0) colors.push(w.color);
      });
      for (var d = 0; d < dots.length; d += 1) {
        dots[d].style.background = colors[d] || "rgba(255,255,255,.22)";
      }
      if (!windows.length) {
        var empty = document.createElement("div");
        empty.className = "empty";
        empty.textContent = I.empty;
        chips.appendChild(empty);
        return;
      }
      var groups = [];
      windows.forEach(function (w) {
        var g = null;
        for (var i = 0; i < groups.length; i += 1) {
          if (groups[i].site_id === w.site_id) { g = groups[i]; break; }
        }
        if (!g) {
          g = { site_id: w.site_id, site_name: w.site_name, color: w.color, items: [] };
          groups.push(g);
        }
        g.items.push(w);
      });
      groups.forEach(function (g) {
        var group = document.createElement("div");
        group.className = "group";
        group.innerHTML = '<span class="gdot"></span><span class="gname"></span>';
        group.querySelector(".gdot").style.background = g.color || "#4f8cff";
        group.querySelector(".gname").textContent = g.site_name || I.noSite;
        group.title = g.site_name || I.noSite;
        chips.appendChild(group);
        g.items.forEach(function (w) { chips.appendChild(chipEl(w)); });
      });
    }

    function allChips() {
      return Array.prototype.slice.call(chips.querySelectorAll(".chip"));
    }
    function select(delta) {
      var all = allChips();
      if (!all.length) return;
      selected = (selected + delta + all.length) % all.length;
      all.forEach(function (r, i) { r.classList.toggle("sel", i === selected); });
      all[selected].scrollIntoView({ block: "nearest", inline: "nearest" });
    }

    // 面板展开时的键盘导航：输入框里有焦点时完全不接管，避免抢 SAS 页面的按键。
    document.addEventListener("keydown", function (e) {
      if (!open) return;
      var tag = document.activeElement && document.activeElement.tagName;
      if (tag && /^(INPUT|TEXTAREA|SELECT)$/.test(tag)) return;
      if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); select(1); }
      else if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); select(-1); }
      else if (e.key === "Enter" && selected >= 0) {
        var c = allChips()[selected];
        if (c) { e.preventDefault(); c.click(); }
      } else if (e.key === "Escape") { hideLook(); setOpen(false); }
    }, true);

    // ---- 状态：初始拉一次 + 事件推送 ----
    function adoptPrefs(p) {
      if (!p || drag || Date.now() < editUntil) return; // 正在拖 / 刚改过：以本地为准，免得回声把滑块拉回去
      var n = {
        cx: clamp(Number(p.cx), 0, 1), cy: clamp(Number(p.cy), 0, 1),
        edge: ["", "top", "bottom", "left", "right"].indexOf(p.edge) >= 0 ? p.edge : "",
        theme: THEME_ORDER.indexOf(p.theme) >= 0 ? p.theme : "midnight",
        bg: isHex(p.bg) ? p.bg : prefs.bg,
        accent: isHex(p.accent) ? p.accent : prefs.accent,
        opacity: clamp(Number(p.opacity), 0.35, 1),
        edge_hide: !!p.edge_hide,
      };
      if (isNaN(n.cx)) n.cx = 0.5;
      if (isNaN(n.cy)) n.cy = 0;
      if (isNaN(n.opacity)) n.opacity = 0.94;
      prefs = n;
    }
    var laidOut = false;
    function relayout() {
      if (!laidOut) {
        // 首次定位不要播放「滑出屏幕」的过渡动画。
        laidOut = true;
        dock.style.transition = "none";
        layout();
        void dock.offsetWidth;
        dock.style.transition = "";
      } else {
        layout();
      }
    }
    window.applyDockState = function (snapshot) {
      if (!snapshot) return;
      var wasPinned = pinned;
      pinned = !!snapshot.pinned;
      windows = Array.isArray(snapshot.windows) ? snapshot.windows : [];
      adoptPrefs(snapshot.prefs);
      applyTheme();
      syncLook();
      pinBtn.classList.toggle("on", pinned);
      pinBtn.title = pinned ? I.unpin : I.pin;
      render();
      // 只在钉住状态真的翻转时改显隐：否则每次焦点切换事件都会把用户正在看的面板收起来。
      if (pinned && !wasPinned) { clearTimeout(hideTimer); setOpen(true); }
      else if (!pinned && wasPinned) { scheduleHide(); }
      relayout();
    };

    applyTheme();
    relayout();
    invoke("get_dock_state").then(window.applyDockState).catch(function () {});
    try {
      if (window.__TAURI__.event && typeof window.__TAURI__.event.listen === "function") {
        window.__TAURI__.event.listen("windows-changed", function (ev) {
          window.applyDockState(ev.payload);
        });
      }
    } catch (e) { /* 没有事件权限时至少保留初始快照 */ }

    // 无边框模式下自绘标题条占顶部 28px，把悬浮栏的可用区域下移避开它（Rust 在 page-load 时会调用）。
    var origShow = window.__sasShowFramelessBar;
    window.__sasShowFramelessBar = function (v, title) {
      if (origShow) { try { origShow(v, title); } catch (e) {} }
      barInset = v ? 28 : 0;
      layout();
    };

    // 原生钩子（Windows）与注入脚本的 keydown 兜底（Linux）都调这个入口。
    window.__sasToggleDock = function () {
      clearTimeout(hideTimer);
      setOpen(!open);
      if (open) { selected = -1; select(1); }
    };
  }

  if (document.documentElement) boot();
  else document.addEventListener("DOMContentLoaded", boot, { once: true });
})();
