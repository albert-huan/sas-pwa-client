// SAS PWA 客户端：内置 WebView 承载 SAS Viya（SAS Studio）站点，不掉线。
//
// 站点地址不再写死在代码里（不同公司域名不同）：
//   * 首次启动弹出「设置」窗口，由用户填写 SAS 站点地址并保存；
//   * 配置持久化到应用配置目录的 config.json，可随时增删改多个环境；
//   * 每个环境一个独立窗口，分别开启 PWA 伪装 / 活动脉冲 / 无边框；
//   * 托盘菜单按配置动态生成，关闭窗口默认只是隐藏到托盘（会话不断）。
//
// 保持会话在线的两个手段：
//   1) WebView2 启动参数关闭 Chromium 后台节流（BROWSER_ARGS）；
//   2) 注入脚本伪装 display-mode / navigator.standalone，并周期性派发 mousemove。

mod config;
mod credentials;
mod hotkey;
mod permissions;
mod taskbar;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use config::{AppConfig, Site};
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, MenuEvent};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewWindowBuilder};

const WINDOW_PREFIX: &str = "site-";
const SETTINGS_LABEL: &str = "settings";
const TRAY_ID: &str = "main-tray";

// Windows 专用 WebView2 启动参数：关闭 Chromium 的后台节流，避免窗口隐藏到托盘后
// 长轮询中断、会话被服务端判定为「遗弃」而销毁。
// 注意：自定义 args 会覆盖 wry 的默认值，必须一并补回默认的 --disable-features。
// （WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS 环境变量在 Tauri 2.11 不生效，只能走该 API。）
const BROWSER_ARGS: &str = "--disable-background-timer-throttling \
    --disable-backgrounding-occluded-windows \
    --disable-renderer-backgrounding \
    --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

// 注入到各 SAS 站点窗口的脚本。占位符由 `render_script` 替换，避免用 format! 处理花括号。
const SAS_INIT_SCRIPT: &str = r#"
(function(){
  try {
    if (__KEEP_AWAKE__) {
      // 「页面始终可见」伪装：WebKitGTK（GTK3）在窗口被最小化 / 隐藏（含本客户端「关闭 = 隐藏到
      // 托盘」）时会把页面标成 hidden 并派发 visibilitychange，SAS 前端收到后往往重新拉数据、
      // 把表格从第一行重绘一遍 —— 表现出来的就是「切个程序回来整页刷新了一次」。
      // Windows 侧我们用 --disable-backgrounding-occluded-windows 等参数关掉了同类行为，
      // 这里对页面做等价伪装：恒报 visible，并丢弃 visibilitychange 监听（只丢这一个事件类型）。
      try {
        Object.defineProperty(document, 'hidden', {
          get: function(){ return false; }, configurable: true
        });
        Object.defineProperty(document, 'visibilityState', {
          get: function(){ return 'visible'; }, configurable: true
        });
        Object.defineProperty(document, 'onvisibilitychange', {
          get: function(){ return null; }, set: function(){}, configurable: true
        });
        var _addEL = EventTarget.prototype.addEventListener;
        EventTarget.prototype.addEventListener = function(type){
          if (type === 'visibilitychange') return;
          return _addEL.apply(this, arguments);
        };
      } catch (e) {}
      var _mm = window.matchMedia;
      window.matchMedia = function(q){
        if (q && typeof q === 'string') {
          var m = q.match(/display-mode:\s*(\w+)/);
          if (m) {
            var mode = m[1];
            // 注意：browser 必须返回 false，否则「普通浏览器」判定成立，反而启用空闲超时。
            var like = (mode === 'standalone' || mode === 'minimal-ui' || mode === 'fullscreen' || mode === 'window-controls-overlay');
            return { matches: like, media: q, onchange: null,
              addListener: function(){}, removeListener: function(){},
              addEventListener: function(){}, removeEventListener: function(){},
              dispatchEvent: function(){ return false; } };
          }
        }
        return _mm ? _mm.call(window, q) : { matches:false, media:q, addListener:function(){}, removeListener:function(){}, addEventListener:function(){}, removeEventListener:function(){}, dispatchEvent:function(){return false;} };
      };
      Object.defineProperty(navigator, 'standalone', { get: function(){ return true; }, configurable: true });
    }
    if (__PULSE_MS__ > 0) {
      setInterval(function(){
        try { window.dispatchEvent(new MouseEvent('mousemove', { view: window, bubbles: true, cancelable: true, clientX: 1, clientY: 1 })); } catch(e){}
      }, __PULSE_MS__);
    }
    // 登录页自动填充：凭据由 Rust 侧解密后注入（磁盘上是 Windows DPAPI 密文）。
    // 只在出现密码输入框时动作（即登录页），已经填过值的框不覆盖用户手输。
    (function(){
      var U = __CRED_USER__, P = __CRED_PASS__, AUTO = __CRED_AUTO__;
      if (!U && !P) return;
      // 自动提交在同一标签页会话里只做一次，避免密码错误反复重试把账号锁掉。
      if (AUTO) {
        try {
          if (sessionStorage.getItem('__sas_autologin__')) AUTO = false;
          else sessionStorage.setItem('__sas_autologin__', '1');
        } catch(e){ AUTO = false; }
      }
      function pick(){
        var pw = document.querySelector('input[type=password]');
        if (!pw) return null;
        var scope = pw.form || document;
        var user = scope.querySelector('input[name=username], input[id=username], input[name=j_username], input[type=text], input[type=email]');
        return { pw: pw, user: user, scope: scope };
      }
      function put(el, v){
        if (!v || !el || el.value) return;
        el.value = v;
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
      }
      function fill(){
        var f = pick();
        if (!f) return false;
        put(f.user, U);
        put(f.pw, P);
        if (AUTO) {
          var btn = f.scope.querySelector('button[type=submit], input[type=submit], #submit, button');
          if (btn) setTimeout(function(){ try { btn.click(); } catch(e){} }, 400);
        }
        return true;
      }
      if (fill()) return;
      var tries = 0;
      var timer = setInterval(function(){
        tries += 1;
        // 登录页可能是跳转或异步渲染出来的：最多轮询 20 秒。
        if (fill() || tries > 40) clearInterval(timer);
      }, 500);
    })();
    var BAR_ID = 'sas-frameless-bar';
    function ensureBar(){
      var bar = document.getElementById(BAR_ID);
      if (bar) return bar;
      bar = document.createElement('div');
      bar.id = BAR_ID;
      bar.setAttribute('data-tauri-drag-region', '');
      bar.style.cssText = 'position:fixed;top:0;left:0;right:0;height:28px;z-index:2147483647;display:none;background:rgba(20,20,25,0.55);color:#fff;font:12px/28px system-ui,"Noto Sans CJK SC","Source Han Sans SC","Microsoft YaHei","WenQuanYi Micro Hei",sans-serif;user-select:none;';
      var title = document.createElement('span');
      title.id = BAR_ID + '-title';
      title.setAttribute('data-tauri-drag-region', '');
      title.textContent = ' ' + (document.title || '');
      title.style.cssText = 'padding-left:10px;';
      var mk = function(label, tip, fn){
        var b = document.createElement('button');
        b.textContent = label;
        if (tip) b.title = tip;
        b.style.cssText = 'float:right;height:28px;width:34px;border:0;background:transparent;color:#fff;cursor:pointer;font-size:13px;';
        b.addEventListener('click', fn);
        return b;
      };
      var win = function(){
        return window.__TAURI__ && window.__TAURI__.window ? window.__TAURI__.window.getCurrentWindow() : null;
      };
      bar.appendChild(title);
      // ×：隐藏到托盘（窗口不可见，但页面继续跑、保活脉冲继续）—— 与系统标题栏 × 行为一致。
      bar.appendChild(mk('×', '隐藏到托盘（保持会话）', function(){ var w = win(); if (w) w.hide(); }));
      // ✕：彻底关闭（销毁窗口、释放 WebView）。cookie 仍在，下次打开还是登录态，
      // 但该窗口的页面状态与保活脉冲会消失 —— 刻意与 × 分开，避免误点。
      bar.appendChild(mk('✕', '彻底关闭窗口', function(){ if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('close_window'); }));
      bar.appendChild(mk('–', '最小化', function(){ var w = win(); if (w) w.minimize(); }));
      bar.appendChild(mk('⚙', '设置', function(){ if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('show_settings'); }));
      document.documentElement.appendChild(bar);
      return bar;
    }
    ensureBar();
    document.addEventListener('DOMContentLoaded', function(){
      var t = document.getElementById(BAR_ID + '-title');
      if (t) t.textContent = ' ' + (document.title || '');
    });
    window.__sasShowFramelessBar = function(v, title){
      var bar = ensureBar();
      bar.style.display = v ? 'block' : 'none';
      if (title) { var t = document.getElementById(BAR_ID + '-title'); if (t) t.textContent = ' ' + title; }
    };
    // F11：切换无边框（去掉 / 恢复系统标题栏），由 Rust 侧写回该站点配置。
    // 正常情况由 Rust 的原生钩子接管（WebView2 会把 F11 当浏览器加速键，这里未必收得到），
    // 本监听是钩子装不上时的兜底。
    // 必须在捕获阶段拦下，否则 SAS 页面或 WebView 自带的全屏会先吃掉这个按键。
    window.addEventListener('keydown', function(e){
      if (e.key === 'F11' || e.keyCode === 122) {
        e.preventDefault();
        e.stopPropagation();
        if (e.repeat) return; // 长按的重复事件不重复切换
        try { window.__TAURI__.core.invoke('toggle_frameless'); }
        catch(err){ console.error('[SAS PWA] F11', err); }
      }
    }, true);
    console.log('[SAS PWA] ready');
  } catch(e){ console.error('[SAS PWA] error', e); }
})();
"#;

// 内存中的配置镜像，读写均通过 config 模块持久化。
static CONFIG: OnceLock<Mutex<AppConfig>> = OnceLock::new();
fn cfg_state() -> &'static Mutex<AppConfig> {
    CONFIG.get_or_init(|| Mutex::new(AppConfig::default()))
}

// 最近一次激活的窗口标签，供托盘左键切换显隐 / 重载使用。
static ACTIVE_WINDOW: OnceLock<Mutex<String>> = OnceLock::new();
fn active_window() -> &'static Mutex<String> {
    ACTIVE_WINDOW.get_or_init(|| Mutex::new(String::new()))
}

// 同一次 F11 去重：原生钩子与注入脚本都会捕获这个键（钩子装不上时靠脚本兜底），
// 按住不放还会持续产生 keydown。记录「窗口 + 时间」，短时间内的重复请求直接忽略。
static LAST_TOGGLE: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
const TOGGLE_DEBOUNCE: Duration = Duration::from_millis(400);

fn window_label(site_id: &str) -> String {
    format!("{WINDOW_PREFIX}{site_id}")
}

// ---------------- 多窗口支持 ----------------
//
// 同一个站点可以同时开多个窗口：所有窗口共用同一份 WebView 数据目录（Windows 是
// WebView2 的 user data dir，Linux 是同一个 WebKit WebContext），cookie / SSO 会话
// 是同一份 —— 新窗口打开就是当前登录身份，等价于 Edge PWA 在同一 profile 下多开。
//
// 标签规则：首个窗口仍是 `site-<id>`，之后依次 `site-<id>-2`、`site-<id>-3`…
// 归属只认下面的注册表，**不靠标签字符串反推**：站点 id 由 sanitize 生成，本身
// 就可能以 `-2` 结尾（如 id = `viya-2`），字符串反推会张冠李戴。

/// 窗口标签 → 站点 id。
static WINDOW_SITES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
fn window_sites() -> &'static Mutex<HashMap<String, String>> {
    WINDOW_SITES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 窗口标签里的序号：`site-<id>` = 1、`site-<id>-2` = 2…（识别不出来按 1 处理）。
fn window_index(label: &str, site_id: &str) -> usize {
    label
        .strip_prefix(WINDOW_PREFIX)
        .and_then(|rest| rest.strip_prefix(site_id))
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(1)
}

/// 某站点的第 n 个窗口标签（n 从 1 开始）。
fn window_label_at(site_id: &str, n: usize) -> String {
    if n <= 1 {
        window_label(site_id)
    } else {
        format!("{WINDOW_PREFIX}{site_id}-{n}")
    }
}

/// 标签 → 站点 id：先查注册表，再退回「恰好等于 site-<id>」的精确匹配。
fn site_id_of_label(label: &str) -> Option<String> {
    if !label.starts_with(WINDOW_PREFIX) {
        return None;
    }
    if let Some(id) = window_sites().lock().unwrap().get(label).cloned() {
        return Some(id);
    }
    get_site(label.strip_prefix(WINDOW_PREFIX)?).map(|s| s.id)
}

/// 挑一个没被占用的标签（多开时依次用 -2、-3…）。
///
/// 注册表里已登记但窗口还没建出来的标签也算占用：连续快速点两次「新窗口」时，
/// 两个后台线程可能同时走到这里，靠「先占位再创建」避免抢同一个标签。
fn free_window_label(app: &tauri::AppHandle, site_id: &str) -> String {
    let mut n = 1usize;
    loop {
        let candidate = window_label_at(site_id, n);
        let reserved = window_sites().lock().unwrap().contains_key(&candidate);
        if !reserved && app.get_webview_window(&candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

/// 某站点当前所有窗口的标签，按窗口序号排序。
fn site_window_labels(app: &tauri::AppHandle, site_id: &str) -> Vec<String> {
    let mut labels: Vec<(usize, String)> = app
        .webview_windows()
        .keys()
        .filter(|label| site_id_of_label(label).as_deref() == Some(site_id))
        .map(|label| (window_index(label, site_id), label.clone()))
        .collect();
    labels.sort();
    labels.into_iter().map(|(_, label)| label).collect()
}

/// 打开站点时优先聚焦「最近用过的那个窗口」，其次取序号最小的。
fn active_or_first_window(app: &tauri::AppHandle, site_id: &str) -> Option<String> {
    let labels = site_window_labels(app, site_id);
    if labels.is_empty() {
        return None;
    }
    let active = active_window().lock().unwrap().clone();
    if labels.contains(&active) {
        return Some(active);
    }
    labels.into_iter().next()
}

/// 窗口标题：同一站点的第 2 个窗口起加序号，任务栏 / 自绘标题条里能区分。
fn window_title(site: &Site, label: &str) -> String {
    let n = window_index(label, &site.id);
    if n <= 1 {
        site.name.clone()
    } else {
        format!("{} ({n})", site.name)
    }
}

/// 站点配色（按站点 id 哈希取固定调色板里的一个）—— 用于窗口图标的标记色。
fn site_color(site_id: &str) -> (u8, u8, u8) {
    const PALETTE: [(u8, u8, u8); 8] = [
        (79, 140, 255),  // 蓝
        (53, 196, 107),  // 绿
        (255, 159, 64),  // 橙
        (229, 83, 83),   // 红
        (166, 102, 255), // 紫
        (38, 198, 218),  // 青
        (233, 196, 0),   // 黄
        (255, 105, 180), // 粉
    ];
    // FNV-1a：站点 id 是 sanitize 过的小写 id，稳定、无依赖
    let mut h: u32 = 2_166_136_261;
    for b in site_id.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    PALETTE[(h as usize) % PALETTE.len()]
}

/// 站点窗口图标：应用图标缩放到 32×32，底部叠一条按站点 id 取色的色带 + 右上小色块。
/// 配合每窗口独立 AUMID，任务栏上就能一眼区分是哪个站点（拿不到应用图标时退化为纯色方块）。
fn site_window_icon(app: &tauri::AppHandle, site: &Site) -> tauri::image::Image<'static> {
    const SIZE: u32 = 32;
    let (r, g, b) = site_color(&site.id);
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    // 底色：站点色（应用图标缺失或带透明时也能看清）
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[r, g, b, 255]);
    }
    if let Some(base) = app.default_window_icon() {
        let (bw, bh) = (base.width().max(1), base.height().max(1));
        let src = base.rgba();
        for y in 0..SIZE {
            for x in 0..SIZE {
                let si = (((y * bh / SIZE) * bw + (x * bw / SIZE)) * 4) as usize;
                let di = ((y * SIZE + x) * 4) as usize;
                if si + 4 <= src.len() {
                    rgba[di..di + 4].copy_from_slice(&src[si..si + 4]);
                }
            }
        }
    }
    // 底部 5px 色带 + 右上 6×6 色块：前者在任务栏小图标下也认得出，后者用于区分相近的色。
    let mut paint = |x0: u32, y0: u32, x1: u32, y1: u32| {
        for y in y0..y1.min(SIZE) {
            for x in x0..x1.min(SIZE) {
                let i = ((y * SIZE + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    };
    paint(0, SIZE - 5, SIZE, SIZE);
    paint(SIZE - 6, 0, SIZE, 6);
    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

fn get_site(id: &str) -> Option<Site> {
    cfg_state()
        .lock()
        .unwrap()
        .sites
        .iter()
        .find(|s| s.id == id)
        .cloned()
}

/// 把任意文本安全地嵌进注入脚本（当作 JS 字符串字面量）。
fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

/// 系统语言标签（如 `zh-CN` / `en-US`）：设置页「跟随系统」据此选中文还是英文。
///
/// 为什么要由 Rust 侧取：WebView2 里的 `navigator.language` 未必等于系统显示语言，
/// 中文系统下也可能报 `en-US`。这里用系统 API 取「用户界面语言」，创建设置窗口时注入
/// 成 `window.__SAS_SYS_LANG__`，前端只做前缀判断（`zh*` → 中文）。
#[cfg(windows)]
fn system_lang_tag() -> String {
    #[link(name = "kernel32")]
    extern "system" {
        /// 当前用户的界面语言（LANGID）。
        fn GetUserDefaultUILanguage() -> u16;
        /// LANGID → 语言标签（如 "zh-CN"）。
        fn LCIDToLocaleName(locale: u32, name: *mut u16, name_count: i32, flags: u32) -> i32;
        /// 界面语言取不到时的兜底：用户的区域设置名。
        fn GetUserDefaultLocaleName(name: *mut u16, name_count: i32) -> i32;
    }

    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    let mut buf = [0u16; LOCALE_NAME_MAX_LENGTH];
    let mut len = unsafe {
        LCIDToLocaleName(
            GetUserDefaultUILanguage() as u32,
            buf.as_mut_ptr(),
            buf.len() as i32,
            0,
        )
    };
    if len <= 0 {
        len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    }
    // 返回值是含结尾 NUL 的字符数；<= 1 说明只有 NUL 或失败。
    if len <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(len as usize - 1)])
}

/// 非 Windows：语言在环境变量里（`LANG=zh_CN.UTF-8` 等）；取不到就返回空串，
/// 由前端退回 `navigator.language`。
#[cfg(not(windows))]
fn system_lang_tag() -> String {
    for key in ["LC_ALL", "LC_MESSAGES", "LC_CTYPE", "LANG"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    String::new()
}

// ---------------- Linux / WebKitGTK 适配 ----------------

/// Linux 运行期适配：**必须在 Tauri（WebKitGTK）初始化之前调用**。
///
/// 1) 渲染路径（流畅度的关键）：WebKitGTK 2.42+ 的「加速合成」只走 DMA-BUF —— 关掉
///    `WEBKIT_DISABLE_DMABUF_RENDERER=1` 就等于退回「Web 进程用 CPU 画共享内存位图、
///    UI 进程再贴图」的非加速路径（见 WebKit 官方 Graphics 文档），滚动与动画都会明显变卡。
///    所以有显卡可用时要保持默认路径；只有已知会花屏 / 闪烁 / 白屏（NVIDIA 专有驱动、WSLg）
///    或**根本没有显卡**（没有 DRM 设备节点）时才降级。环境变量必须在这里设好，
///    之后 WebKitGTK 初始化才读得到。
///    - `smooth`：保持默认（DMA-BUF 加速）+ 强制加速合成，最快；
///    - `compat`：强制关闭 DMA-BUF，用于花屏/闪烁/黑屏时救急；
///    - `auto`   ：按环境自动判定 —— WSL / NVIDIA 专有驱动关 DMA-BUF；**没有 GPU 加速
///      （`/dev/dri` 里没有 `renderD*` 渲染节点，只有 `card*` 的 BMC 2D 芯片也算）时再额外
///      关掉加速合成**：那种机器上加速合成只能跑在 llvmpipe 软件 GL 上，白白多一层拷贝与
///      GL 开销，纯 CPU 合成通常更稳更快（想验证反面效果就用「流畅优先」）；
///    - 用户自己设过同名环境变量时一律以用户为准，方便现场逐个试参数。
/// 2) 中文字体：缺字体时中文会渲染成方块，应用侧装不了字体，只能在日志里留一条可执行的建议。
/// 3) 诊断：把会话类型 / 显卡 / 设备节点 / 驱动版本 / 最终生效的渲染路径写进日志，远程排查时
///    能一眼看出对方是不是跑在软件渲染或降级路径上。
///
/// 返回值是一行诊断信息，写进 `debug.log`。
#[cfg(target_os = "linux")]
fn tune_linux_webkit(mode: &str) -> String {
    let mut notes: Vec<String> = Vec::new();

    let wsl = std::fs::read_to_string("/proc/version")
        .map(|v| v.to_lowercase().contains("microsoft"))
        .unwrap_or(false);
    let nvidia = std::fs::read_to_string("/proc/driver/nvidia/version")
        .ok()
        .and_then(|line| first_version_token(&line));
    // 有没有可用的显卡设备：Some(false) = 确定没有（纯软件渲染），None = 判断不了（读不到 /dev/dri）。
    let gpu = gpu_available();
    let software_only = gpu == Some(false);

    // ---- 渲染路径：决定加速合成能不能用（这是 Linux 上「卡不卡」的头号因素）----
    let user_set = std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER");
    let disable = match mode {
        "compat" => true,
        "smooth" => false,
        _ => wsl || nvidia.is_some() || software_only,
    };
    let decision = if user_set.is_some() {
        "沿用环境变量 WEBKIT_DISABLE_DMABUF_RENDERER".to_string()
    } else if disable {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        if software_only {
            "共享内存（无 GPU，软件渲染）".to_string()
        } else {
            "共享内存（非加速，兼容性优先）".to_string()
        }
    } else {
        "DMA-BUF（WebKit 默认加速路径）".to_string()
    };

    let mut causes: Vec<String> = Vec::new();
    if wsl {
        causes.push("WSL".to_string());
    }
    if let Some(v) = nvidia.as_deref() {
        causes.push(format!("NVIDIA 专有驱动 {v}"));
    }
    if software_only {
        causes.push("无 GPU（软件渲染）".to_string());
    }
    let auto_reason = if causes.is_empty() {
        "自动：未发现已知问题环境".to_string()
    } else {
        format!("自动：{}", causes.join(" + "))
    };
    let reason = match mode {
        "compat" => "按「兼容优先」强制降级".to_string(),
        "smooth" => "按「流畅优先」保持加速".to_string(),
        _ if user_set.is_some() => "环境变量优先".to_string(),
        _ => auto_reason,
    };
    notes.push(format!("渲染模式={mode}（{reason}）→ {decision}"));

    // 加速合成的取舍：
    // - 「流畅优先」：强制打开（环境本身不支持时 WebKit 会自己退回，不会因此更差）；
    // - 「自动」+ 无 GPU：加速合成只能跑在 llvmpipe 软件 GL 上，多一层拷贝与 GL 状态开销，
    //   关掉走纯 CPU 合成通常更稳更快 —— 想对比反面就切「流畅优先」。
    // 两种情况都不覆盖用户自己设过的同名变量。
    if mode == "smooth"
        && user_set.is_none()
        && std::env::var_os("WEBKIT_FORCE_COMPOSITING_MODE").is_none()
    {
        std::env::set_var("WEBKIT_FORCE_COMPOSITING_MODE", "1");
        notes.push("已强制开启加速合成（流畅优先）".into());
    } else if mode == "auto"
        && software_only
        && std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        notes.push("无 GPU：已关闭加速合成，走纯 CPU 合成（软渲染下通常更稳更快）".into());
    }

    // ---- 环境探测：只为排查用，不影响行为 ----
    let session = std::env::var("XDG_SESSION_TYPE").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            "wayland".to_string()
        } else if std::env::var_os("DISPLAY").is_some() {
            "x11".to_string()
        } else {
            "无显示（headless?）".to_string()
        }
    });
    let nodes = dri_nodes();
    let has_render_node = nodes.iter().any(|n| n.starts_with("renderD"));
    let dri_desc = if nodes.is_empty() {
        if std::path::Path::new("/dev/dri").exists() {
            "/dev/dri 里没有 card*/renderD* 节点".to_string()
        } else {
            "无 /dev/dri".to_string()
        }
    } else if has_render_node {
        nodes.join(",")
    } else {
        // 只有显示控制器、没有渲染节点：典型是服务器 BMC 的 ASPEED / Matrox 2D 芯片。
        format!("{}（无 renderD* 渲染节点，只有 2D 显示控制器）", nodes.join(","))
    };
    notes.push(format!(
        "会话={session}；GPU={}；显示设备={dri_desc}",
        drm_gpu_summary()
    ));
    if std::path::Path::new("/dev/dxg").exists() {
        notes.push("检测到 /dev/dxg（WSL GPU 直通）".into());
    }
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_CLIENT").is_some() {
        notes.push(
            "检测到 SSH 会话：若通过 X11 转发 / VNC 之类的远程桌面使用，画面必然不如本地流畅".into(),
        );
    }

    let gl_env: Vec<String> = [
        "LIBGL_ALWAYS_SOFTWARE",
        "MESA_LOADER_DRIVER_OVERRIDE",
        "GALLIUM_DRIVER",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
        "WEBKIT_FORCE_COMPOSITING_MODE",
    ]
    .into_iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| format!("{k}={v}")))
    .collect();
    if !gl_env.is_empty() {
        notes.push(format!("GL 相关环境变量：{}", gl_env.join(" ")));
    }

    // 顺畅度的排查建议：不同机器差的环节不一样，日志里给一句能直接照做的。
    if software_only {
        notes.push(
            "本机没有 GPU 加速（/dev/dri 里没有 renderD* 渲染节点）：WebKit 只能软件渲染（llvmpipe），\
             流畅度上限由 CPU 决定。只有 card* 的机器（服务器 BMC 的 ASPEED / Matrox 等 2D 显示芯片）\
             同样属于这种情况；若这台机器本来有显卡，再查内核驱动是否加载、虚拟机是否开了 3D 加速、\
             容器是否映射了 /dev/dri"
                .into(),
        );
        notes.push("可用 WEBKIT_SHOW_FPS=1 启动，在页面右上角看实时帧率做对比".into());
    } else if gpu.is_none() {
        notes.push("读不到 /dev/dri（权限问题？），无法判断是否软件渲染".into());
    } else if !mesa_dri_drivers_present() {
        notes.push(
            "有渲染节点但没找到 mesa 的 DRI 驱动（*_dri.so），GL 仍可能退回 llvmpipe：\
             建议 sudo apt install libgl1-mesa-dri"
                .into(),
        );
    } else if mode == "auto" && disable && user_set.is_none() {
        notes.push(
            "若显示正常但觉得卡顿，可在设置页把「渲染模式」改成「流畅优先」后重启对比".into(),
        );
    }

    match find_cjk_font() {
        Some(font) => notes.push(format!("中文字体：{font}")),
        None => notes.push(
            "未检测到中文字体，中文可能显示为方块，建议：sudo apt install fonts-noto-cjk".into(),
        ),
    }

    notes.join("；")
}

/// 有没有可用的 **渲染节点**（`/dev/dri/renderD*`），也就是能不能真正用上 GPU 加速。
///
/// 判据必须是渲染节点，而不是 `card*`：`card*` 只说明「有个显示控制器」。服务器主板 BMC 上的
/// ASPEED / Matrox 这类 2D 芯片同样会注册 `card*`，但它们没有 3D/GL 能力、也不提供渲染节点
/// （桌面仍会显示 `GPU=llvmpipe`）。只看 `card*` 会把这类机器误判成「有 GPU 可用」。
/// 返回 `None` 表示读不到 `/dev/dri`（例如没权限），这时**不要**降级，避免误伤有卡的机器。
#[cfg(target_os = "linux")]
fn gpu_available() -> Option<bool> {
    match std::fs::read_dir("/dev/dri") {
        Ok(entries) => Some(entries.flatten().any(|e| {
            e.file_name().to_string_lossy().starts_with("renderD")
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// `/dev/dri` 下实际存在的 `card*` / `renderD*` 节点（排序后返回；目录不存在或读不了就是空表）。
#[cfg(target_os = "linux")]
fn dri_nodes() -> Vec<String> {
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/dri")
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("card") || n.starts_with("renderD"))
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
}

/// 有没有装 mesa 的 DRI 驱动（`*_dri.so`）。有渲染节点但没装这套驱动时，GL 仍会退回 llvmpipe。
#[cfg(target_os = "linux")]
fn mesa_dri_drivers_present() -> bool {
    const DIRS: [&str; 3] = ["/usr/lib/x86_64-linux-gnu/dri", "/usr/lib/dri", "/usr/lib64/dri"];
    DIRS.iter().any(|dir| {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries.flatten().any(|e| {
                    e.file_name().to_string_lossy().ends_with("_dri.so")
                })
            })
            .unwrap_or(false)
    })
}

/// 汇总 `/sys/class/drm` 里的显示适配器标识（如 `card0:Intel/iris`），写日志用。
///
/// 为什么走 sysfs：核显 / 独显 / 服务器 BMC 的 2D 芯片 / 虚拟 GPU 都能认出来，且不依赖
/// `lspci` 之类的命令行工具。认不出的厂商会带上原始 PCI vendor id（如 `card1:未知厂商 0x1234`），
/// 内核驱动名取自 `device/uevent` 的 `DRIVER=`，一眼能看出是不是 `ast` / `mgag200` 这类
/// 没有 3D 能力的显示控制器。
#[cfg(target_os = "linux")]
fn drm_gpu_summary() -> String {
    const VENDORS: [(&str, &str); 12] = [
        ("0x8086", "Intel"),
        ("0x1002", "AMD"),
        ("0x1022", "AMD"),
        ("0x10de", "NVIDIA"),
        ("0x1af4", "virtio-gpu"),
        ("0x1b36", "QXL"),
        ("0x15ad", "VMware"),
        ("0x80ee", "VirtualBox"),
        // 服务器 BMC 上的 2D 显示控制器：能显示画面，但没有 3D / GL 能力。
        ("0x1a03", "ASPEED(BMC)"),
        ("0x102b", "Matrox(BMC?)"),
        ("0x1234", "QEMU/Bochs"),
        ("0x1013", "Cirrus"),
    ];

    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return "未知（读不到 /sys/class/drm）".to_string();
    };
    // 只取 cardN 本体，排除 cardN-DP-1 这类显示连接器目录。
    let mut cards: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            name.starts_with("card") && !name.contains('-')
        })
        .collect();
    cards.sort();

    let gpus: Vec<String> = cards
        .iter()
        .map(|path| {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let vendor = std::fs::read_to_string(path.join("device/vendor")).unwrap_or_default();
            let vendor = vendor.trim().to_ascii_lowercase();
            let label: String = VENDORS
                .iter()
                .find(|(id, _)| *id == vendor.as_str())
                .map(|(_, label)| (*label).to_string())
                .unwrap_or_else(|| {
                    // 认不出就带上原始 id（如 0x1234）：回来一查就知道是哪家的虚拟显卡。
                    if vendor.is_empty() {
                        "未知厂商".to_string()
                    } else {
                        format!("未知厂商 {vendor}")
                    }
                });
            match kernel_driver_of(path) {
                Some(driver) => format!("{name}:{label}/{driver}"),
                None => format!("{name}:{label}"),
            }
        })
        .collect();

    if gpus.is_empty() {
        "未识别（sysfs 无 cardN）".to_string()
    } else {
        gpus.join(",")
    }
}

/// 从 `/sys/class/drm/cardN/device/uevent` 里取内核驱动名（`DRIVER=ast` 等）。
#[cfg(target_os = "linux")]
fn kernel_driver_of(card: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(card.join("device/uevent")).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("DRIVER=").map(|d| d.trim().to_string()))
        .filter(|d| !d.is_empty())
}

/// 从一段文本里抠出第一个形如 `535.171.04` 的版本号（NVIDIA 驱动版本行用）。
#[cfg(target_os = "linux")]
fn first_version_token(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(str::to_string)
}

/// 粗查系统里有没有常见中文字体（只看几个标准安装路径，够用来给日志提示）。
#[cfg(target_os = "linux")]
fn find_cjk_font() -> Option<&'static str> {
    const CANDIDATES: [(&str, &str); 7] = [
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "Noto Sans CJK"),
        ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", "Noto Sans CJK"),
        ("/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc", "Noto Serif CJK"),
        (
            "/usr/share/fonts/opentype/source-han-sans/SourceHanSansSC-Regular.otf",
            "Source Han Sans SC",
        ),
        ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", "WenQuanYi Micro Hei"),
        ("/usr/share/fonts/truetype/arphic/uming.ttc", "AR PL UMing"),
        ("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf", "Droid Sans Fallback"),
    ];
    CANDIDATES
        .iter()
        .find(|(path, _)| std::path::Path::new(path).exists())
        .map(|(_, name)| *name)
}

/// 按站点配置渲染注入脚本（含该站点的登录凭据）。
fn render_script(app: &tauri::AppHandle, site: &Site) -> String {
    let pulse = if site.keep_awake {
        site.pulse_seconds.saturating_mul(1000).min(3_600_000)
    } else {
        0
    };

    // 登录凭据：DPAPI 解密后的明文只出现在注入脚本里（不落盘）。
    // http 站点不注入，避免明文密码走明文通道。
    let (user, password, auto_login) = if site.url.starts_with("https://") {
        credentials::login_of(app, &site.id).unwrap_or_default()
    } else {
        (String::new(), String::new(), false)
    };

    SAS_INIT_SCRIPT
        .replace("__KEEP_AWAKE__", if site.keep_awake { "true" } else { "false" })
        .replace("__PULSE_MS__", &pulse.to_string())
        .replace("__CRED_USER__", &js_string(&user))
        .replace("__CRED_PASS__", &js_string(&password))
        .replace("__CRED_AUTO__", if auto_login { "true" } else { "false" })
}

/// 日志滚动阈值：单个 debug.log 超过它就挪成 `debug.log.1` 重新写。
const LOG_MAX_BYTES: u64 = 1024 * 1024;
/// 轮转时保留的上一份日志文件名。只留一份 —— 排查通常只看最近一段，留多了白占空间。
const LOG_BACKUP: &str = "debug.log.1";
/// 每写这么多条才 stat 一次文件大小：`log_line` 在页面每次 navigation / page-load 都会被调用，
/// 逐条 stat 不划算，而滚动本身也不需要那么精确。
const LOG_SIZE_CHECK_EVERY: u64 = 64;

/// 诊断日志：追加到配置目录的 debug.log，便于远程排查（对正常使用无影响）。
///
/// **必须滚动**：本客户端是常驻进程（「关闭」只是隐藏到托盘），而 SAS 页面会不断发请求、
/// 触发 page-load / permission 日志，几个月下来 debug.log 能长到几百 MB，既占磁盘也让
/// 远程取日志变得困难。代价是每 `LOG_SIZE_CHECK_EVERY` 条一次 stat、滚动时一次 rename。
fn log_line(app: &tauri::AppHandle, msg: &str) {
    static WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join("debug.log");

        let n = WRITTEN.fetch_add(1, Ordering::Relaxed);
        if n % LOG_SIZE_CHECK_EVERY == 0 {
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() > LOG_MAX_BYTES {
                    let backup = dir.join(LOG_BACKUP);
                    // 上一轮备份删不掉（被占用？）时 rename 也会失败，那就继续追加，不致命。
                    let _ = std::fs::remove_file(&backup);
                    if std::fs::rename(&path, &backup).is_ok() {
                        log_append(
                            &path,
                            &format!(
                                "[{secs}] debug.log 已达 {} 字节，已滚动为 {LOG_BACKUP}",
                                meta.len()
                            ),
                        );
                    }
                }
            }
        }
        log_append(&path, &format!("[{secs}] {msg}"));
    }
}

/// 往日志文件追加一行（文件不存在就创建）。滚动失败、并发 rename 撞车都只会丢这一行，不影响主流程。
fn log_append(path: &std::path::Path, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = std::io::Write::write_fmt(&mut f, format_args!("{line}\n"));
    }
}

/// 应用无边框外观（含自绘标题条显隐）。仅在需要改变时才动 decorations。
///
/// 一个站点可能开了多个窗口，逐个应用（标题条文案用各自窗口的标题，带序号）。
fn apply_frameless(app: &tauri::AppHandle, site: &Site) {
    for label in site_window_labels(app, &site.id) {
        if let Some(w) = app.get_webview_window(&label) {
            let decorated = w.is_decorated().unwrap_or(true);
            if decorated == site.frameless {
                let _ = w.set_decorations(!site.frameless);
            }
            let script = format!(
                "if(window.__sasShowFramelessBar)window.__sasShowFramelessBar({}, {});",
                site.frameless,
                js_string(&window_title(site, &label))
            );
            let _ = w.eval(&script);
        }
    }
}

fn focus_window(app: &tauri::AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    let mut active = active_window().lock().unwrap();
    *active = label.to_string();
}

/// 彻底关闭一个窗口：销毁 WebView、释放内存（与 `hide()`「收进托盘继续保活」相对）。
///
/// 用 `destroy()` 而不是 `close()`：tauri 2.11 里 `destroy()` 不派发任何事件、强制关闭，
/// 因此不会被 `CloseRequested` 的「隐藏」逻辑拦下；收尾（注册表、当前窗口、托盘）统一由
/// `Destroyed` 事件负责，调用方不用自己清。
fn destroy_site_window(app: &tauri::AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        log_line(app, &format!("destroy {label}"));
        if let Err(e) = w.destroy() {
            eprintln!("[SAS PWA 客户端] 彻底关闭窗口失败：{e}");
            log_line(app, &format!("destroy {label} failed: {e}"));
        }
    } else {
        // 窗口已经不在（可能刚被关掉）：顺手清掉注册表里的孤儿条目。
        window_sites().lock().unwrap().remove(label);
    }
}

/// 判断两个 URL 是否指向同一站点（scheme + host + port 一致即视为同站）。
fn same_site(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// 【关键规则】窗口创建必须由事件循环执行（也就是必须在非主线程发起）。
///
/// 依据 tauri-runtime-wry 的 `Context::create_window`：从主线程调用时创建逻辑会**就地执行**，
/// 而同步命令恰好运行在主线程的 IPC 回调栈里（WebView2 的 WebMessageReceived 处理中），
/// 在该回调内同步初始化新的 WebView2 会死锁 —— 表现为「点打开毫无反应、设置窗口显示未响应、
/// 进程还活着」，debug.log 里 `creating` 之后永远等不到 `created`。
/// 同理，setup 阶段也不能就地创建**远程**站点窗口：那时事件循环还没开始派发消息，
/// WebView2 初始化同样永远等不到完成。因此：
///   * `open_site` 命令声明为 async（在线程池上执行，不是主线程）；
///   * 托盘菜单 / 单实例回调里用 std::thread::spawn 发起创建；
///   * 启动恢复默认站点也用 std::thread::spawn（请求入队，事件循环跑起来后执行）；
///   * 只有加载本地 index.html 的设置窗口可以在 setup 里就地创建（已实测成功）。

/// 新建 SAS 站点窗口（只能在主线程之外调用，理由见上方【关键规则】）。
/// `label` 由调用方给出：多开时是 `site-<id>-2`…（见 `free_window_label`）；
/// `target` 一般是站点地址，页面自己 `window.open(url)` 时可以是它要求的同源地址。
fn create_site_window(
    app: &tauri::AppHandle,
    site: &Site,
    label: &str,
    target: url::Url,
) -> Result<(), String> {
    // 是否允许在这个站点窗口里打开 DevTools。默认关闭，理由见 `AppConfig::dev_tools`：
    // 窗口里跑的是远程 SAS 页面，开着 DevTools 等于把注入脚本里的登录凭据摆出来。
    let dev_tools = cfg_state().lock().unwrap().dev_tools;
    // 同一站点的第 2 个窗口起标题带序号，任务栏里能区分。
    let title = window_title(site, label);
    log_line(app, &format!("creating {label} -> {target}"));

    // 防白屏：窗口先隐藏创建，等页面真正加载完成（on_page_load Finished）再显示，
    // 否则 Windows/WebView2 常把「首帧空白」直接呈现给用户。
    let shown = Arc::new(AtomicBool::new(false));
    let shown_cb = shown.clone();
    let frameless = site.frameless;
    let site_name = title.clone();
    // 供 on_new_window 回调（'static）使用
    let new_win_app = app.clone();
    let new_win_site = site.clone();

    // 先占位再创建（连续点两次「新窗口」不会抢同一个标签），创建失败时回滚。
    window_sites()
        .lock()
        .unwrap()
        .insert(label.to_string(), site.id.clone());

    let built =
        WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::External(target.clone()))
            .title(title)
            .initialization_script(render_script(app, site))
            .additional_browser_args(BROWSER_ARGS)
            // 【必须关掉 Tauri 的拖放处理器】它在 Windows 上会对 WebView2 调
            // `ICoreWebView2Controller4::SetAllowExternalDrop(false)`（见 wry webview2/mod.rs），
            // 页面内的 HTML5 拖放就废了 —— SAS Studio「打开的程序 / 数据」的标签拖动排序正是这套
            // dragstart/dragover/drop API（Tauri 文档原话：Windows 上要用 HTML5 拖放必须关掉它）。
            // 我们并不使用 Tauri 的文件拖放事件，关掉后拖放回到浏览器默认行为，与 Edge 一致。
            .disable_drag_drop_handler()
            // Ctrl+滚轮 / 触控板缩放（wry 默认是关的，Edge PWA 里能用）
            .zoom_hotkeys_enabled(true)
            // 剪贴板读取：不开这个开关，wry 不会把 CLIPBOARD_READ 权限自动放行，
            // 页面上的「粘贴」类按钮（navigator.clipboard.readText）会被拒。
            .enable_clipboard_access()
            // 页面里 target="_blank" / window.open() 的落点：不交给 WebView2 的默认弹窗
            // （那个窗口没有注入脚本、没有 __TAURI__、也不在托盘与窗口注册表里），
            // 改走我们自己的新建窗口流程 —— 保活 / PWA 伪装 / 无边框 / 编号标签全都一致。
            // 注意：这个回调跑在 WebView2 的事件回调（主线程）里，创建窗口必须丢后台线程，
            // 否则死锁（见文件顶部【关键规则】）。
            .on_new_window(move |url, _features| {
                let app = new_win_app.clone();
                let site = new_win_site.clone();
                std::thread::spawn(move || {
                    if let Err(e) = create_new_site_window_at(&app, &site, Some(url)) {
                        log_line(&app, &format!("page new window failed: {e}"));
                    }
                });
                tauri::webview::NewWindowResponse::Deny
            })
            .devtools(dev_tools)
            .inner_size(1280.0, 800.0)
            .min_inner_size(900.0, 600.0)
            .decorations(!site.frameless)
            .resizable(true)
            .center()
            .visible(false)
            .on_page_load(move |w, payload| {
                log_line(
                    w.app_handle(),
                    &format!(
                        "page-load {}: {:?} {}",
                        w.label(),
                        payload.event(),
                        payload.url()
                    ),
                );
                if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                    return;
                }
                // 页面就绪后再按配置显隐自绘标题条（创建时立即 eval 往往还取不到注入函数）。
                let script = format!(
                    "if(window.__sasShowFramelessBar)window.__sasShowFramelessBar({}, {});",
                    frameless,
                    serde_json::to_string(&site_name).unwrap_or_else(|_| "\"\"".into())
                );
                let _ = w.eval(&script);
                if !shown_cb.swap(true, Ordering::SeqCst) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            })
            .build()
            .map_err(|e| {
                window_sites().lock().unwrap().remove(label);
                log_line(app, &format!("create {label} failed: {e}"));
                format!("创建窗口失败：{e}")
            })?;
    log_line(
        app,
        &format!(
            "created {label}, url now: {}",
            built
                .url()
                .map(|u| u.to_string())
                .unwrap_or_else(|e| format!("<err:{e}>"))
        ),
    );
    // 权限请求接管（仅 Windows）：网页通知 / 剪贴板读取 / 多文件下载放行，其余记为日志。
    // 必须在 webview 建好之后挂，所以放这里（窗口还没 show，页面也还没开始请求权限）。
    if let Err(e) = permissions::install(&built, app.clone()) {
        log_line(app, &format!("install permission handler {label} failed: {e}"));
    }
    // 站点专属窗口图标（任务栏上区分站点）：放这里而不是 builder 链上 ——
    // `WebviewWindowBuilder::icon()` 返回 `Result`，会把 builder 链打断；而窗口此时还没 show，
    // 任务栏按钮尚未创建，先设好后效果完全一样。图标只影响观感，失败不打断创建。
    if let Err(e) = built.set_icon(site_window_icon(app, site)) {
        log_line(app, &format!("set icon {label} failed: {e}"));
    }
    // 任务栏身份：每站点一个 AUMID → 任务栏上独立分组 / 独立图标。
    // **必须在 show 之前设置**（Windows 在窗口变可见时才定格任务栏按钮），此刻窗口还是隐藏的。
    #[cfg(windows)]
    if let Ok(hwnd) = built.hwnd() {
        match std::env::current_exe() {
            Ok(exe) => {
                if let Err(e) = taskbar::apply_site_identity(hwnd.0, &site.id, &site.name, &exe) {
                    log_line(app, &format!("taskbar identity {label} failed: {e}"));
                }
            }
            Err(e) => log_line(app, &format!("current_exe failed: {e}")),
        }
    }
    // 「标签 → 站点」在创建前就已登记（见上方占位说明），这里不再重复。
    // 不 show，等页面加载完成再显示（见 on_page_load）；只先登记为当前窗口。
    {
        let mut active = active_window().lock().unwrap();
        *active = label.to_string();
    }
    apply_frameless(app, site);

    // 兜底：
    //  1) 1.5s 后确认导航真的发生了 —— 没发生（还停在 about:blank）才补一次导航，防白屏；
    //     已经跳走到任何 http(s) 地址都不要干预（详见下面第一个 if 的注释：SSO）。
    //  2) 再过 2s 无论页面有没有触发 Finished（证书错误 / 登录跳转卡住）都强制显示，
    //     否则窗口一直隐藏，看起来就像「点了打开没反应」。
    let handle = app.clone();
    let lbl = label.to_string();
    let tgt = target.clone();
    let shown_thread = shown.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if let Some(w) = handle.get_webview_window(&lbl) {
            let cur = w
                .url()
                .map(|u| u.to_string())
                .unwrap_or_else(|e| format!("<err:{e}>"));
            log_line(&handle, &format!("post-check {lbl}: url={cur}"));
            // 判据是「有没有真的导航过」，而不是「在不在目标站」—— 这一点很关键：
            // SAS 站点普遍走 SSO，打开站点后 WebView 可能被重定向到**另一个 host** 的登录页
            // （外部 IdP、独立部署的 SASLogon 等）。当初这里按「地址与目标站不符 = 首次导航丢了」
            // 直接 navigate 回站点首页，结果是：① 用户正在填的登录表单被整页刷新冲掉；
            // ② IdP 回跳后又被拉回来，来回重定向形成登录死循环。
            //
            // 导航真的没发生时，地址仍会是 about:blank（不是 http/https），只有这种情况才需要补导航。
            // 页面处在 SSO 跳转链的哪一环都算「正常工作」，别碰它的地址。
            let scheme = url::Url::parse(&cur)
                .map(|u| u.scheme().to_ascii_lowercase())
                .unwrap_or_default();
            if scheme == "http" || scheme == "https" {
                // 已经落在某个页面上了（包括 SSO 跳转的中间态）：只保证窗口被显示出来，
                // 不等 Finished（有些站点一直挂着重定向，永远等不到）。
                if !shown_thread.swap(true, Ordering::SeqCst) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            } else {
                log_line(
                    &handle,
                    &format!("post-check 尚未发生导航（url={cur}） -> navigate {tgt}"),
                );
                let _ = w.navigate(tgt);
            }
            std::thread::sleep(std::time::Duration::from_millis(2000));
            if !shown_thread.load(Ordering::SeqCst) {
                log_line(&handle, &format!("fallback show {lbl}"));
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
    });
    Ok(())
}

/// 打开（或聚焦）某个 SAS 站点窗口：已有窗口就聚焦（同一站点开了多个时优先最近用过的
/// 那个），一个都没有才新建。
fn open_site_window(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let site = get_site(id).ok_or_else(|| format!("未找到站点：{id}"))?;
    let target = url::Url::parse(&site.url).map_err(|e| e.to_string())?;

    if let Some(label) = active_or_first_window(app, &site.id) {
        log_line(app, &format!("open id={} label={label} url={target}", site.id));
        if let Some(w) = app.get_webview_window(&label) {
            // 只有「压根没导航过」（还停在 about:blank）才补一次地址。
            //
            // 不按「地址与目标站不符」来纠偏，理由同 `create_site_window` 的 post-check：
            // 窗口很可能正停在 SSO 跳转链上（外部 IdP 的页面，host 与站点不同），
            // 这时导航过去会把用户正在进行的登录冲掉；而每次点击都整页刷新
            // 又会丢掉会话状态（原注释担心的正是这一点）—— 两种情况都该保守。
            let needs_url = match w.url() {
                Ok(cur) => {
                    let scheme = cur.scheme().to_ascii_lowercase();
                    !(scheme == "http" || scheme == "https")
                }
                Err(_) => true,
            };
            if needs_url {
                let _ = w.navigate(target);
            }
        }
        focus_window(app, &label);
    } else {
        // 直接创建：在命令线程调用时 tauri 会把创建请求派发到事件循环，安全。
        let label = free_window_label(app, &site.id);
        log_line(app, &format!("open id={} label={label} url={target}", site.id));
        create_site_window(app, &site, &label, target)?;
    }

    // 记住最近打开的站点，下次启动自动恢复。
    {
        let mut cfg = cfg_state().lock().unwrap();
        if cfg.last_site_id.as_deref() != Some(site.id.as_str()) {
            cfg.last_site_id = Some(site.id.clone());
            let snapshot = cfg.clone();
            drop(cfg);
            let _ = config::save(app, &snapshot);
        }
    }
    Ok(())
}

/// 为同一站点再开一个窗口（不聚焦已有窗口）。
///
/// 所有站点窗口共用同一份 WebView 数据目录（cookie / SSO 会话），新窗口打开时就是
/// 当前登录身份 —— 等价于 Edge PWA 在同一 profile 下多开窗口。
fn create_new_site_window(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let site = get_site(id).ok_or_else(|| format!("未找到站点：{id}"))?;
    create_new_site_window_at(app, &site, None)
}

/// 同上，但可指定初始地址（页面自己 `window.open(url)` 时用）。
/// 只采信与站点同源的地址：否则外部链接会被塞进一个带注入脚本的站点窗口里。
fn create_new_site_window_at(
    app: &tauri::AppHandle,
    site: &Site,
    requested: Option<url::Url>,
) -> Result<(), String> {
    let base = url::Url::parse(&site.url).map_err(|e| format!("地址无效：{e}"))?;
    let target = match requested {
        Some(u) if same_site(&u, &base) => u,
        _ => base,
    };
    let label = free_window_label(app, &site.id);
    log_line(
        app,
        &format!("new window id={} label={label} url={target}", site.id),
    );
    create_site_window(app, site, &label, target)
}

/// 打开设置窗口（本地 dist 页面）。
fn open_settings(app: &tauri::AppHandle) {
    if app.get_webview_window(SETTINGS_LABEL).is_some() {
        focus_window(app, SETTINGS_LABEL);
        return;
    }
    // 同样直接创建（不要走 run_on_main_thread，理由见文件顶部说明）。
    // 系统语言随窗口注入：i18n.js 的「跟随系统」以此为准（WebView2 的 navigator.language 不可尽信）。
    // DevTools 开关：默认关闭（见 `AppConfig::dev_tools`）。
    let dev_tools = cfg_state().lock().unwrap().dev_tools;
    let lang = system_lang_tag();
    let init_script = format!("window.__SAS_SYS_LANG__ = {};", js_string(&lang));
    log_line(app, &format!("settings window system lang: {lang:?}"));
    let res = (|| -> Result<(), String> {
        let built = WebviewWindowBuilder::new(
            app,
            SETTINGS_LABEL,
            tauri::WebviewUrl::App("index.html".into()),
        )
        // 标题用中英一致的品牌名：窗口一显示就会被前端的 syncWindowTitle() 换成
        // 当前语言的标题，写死中文的话英文界面下会先闪一下中文（中文下两处文案也不同）。
        .title("SAS PWA Client")
        .initialization_script(&init_script)
        .additional_browser_args(BROWSER_ARGS)
        .devtools(dev_tools)
        .inner_size(1120.0, 860.0)
        .min_inner_size(720.0, 560.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| format!("创建设置窗口失败：{e}"))?;
        log_line(
            app,
            &format!(
                "settings window url: {}",
                built
                    .url()
                    .map(|u| u.to_string())
                    .unwrap_or_else(|e| format!("<err:{e}>"))
            ),
        );
        Ok(())
    })();
    if let Err(e) = res {
        log_line(app, &format!("settings window error: {e}"));
    }
    focus_window(app, SETTINGS_LABEL);
}

/// 事件回调（菜单 / 托盘 / 单实例）里打开设置窗口：派发到后台线程创建，
/// 避免在主线程回调里就地初始化 WebView（理由见上方【关键规则】）。
fn open_settings_from_event(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || open_settings(&app));
}

/// 按当前配置重建托盘菜单。
fn refresh_tray(app: &tauri::AppHandle) -> Result<(), String> {
    let menu = build_tray_menu(app)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn build_tray_menu<M: Manager<tauri::Wry>>(app: &M) -> Result<tauri::menu::Menu<tauri::Wry>, String> {
    let sites = cfg_state().lock().unwrap().sites.clone();
    let mut builder = MenuBuilder::new(app);

    // 说明：MenuItemBuilder::build 返回 tauri::Error，统一转成字符串错误。
    let item = |id: &str, text: &str| {
        MenuItemBuilder::with_id(id.to_string(), text.to_string())
            .build(app)
            .map_err(|e| e.to_string())
    };

    if sites.is_empty() {
        builder = builder.item(&item("settings", "打开设置…")?);
    } else {
        for s in &sites {
            // 默认登录环境在菜单里标出来，方便一眼看出启动会连哪台服务器。
            let title = if s.default {
                format!("打开 {}（默认）", s.name)
            } else {
                format!("打开 {}", s.name)
            };
            builder = builder.item(&item(&format!("open:{}", s.id), &title)?);
        }
        builder = builder.separator();
        builder = builder.item(&item("settings", "站点设置")?);

        // 「无边框窗口」勾选项：作用于当前激活的站点窗口，没有站点窗口时置灰。
        let active_label = active_window().lock().unwrap().clone();
        // 多开时标签可能带序号，站点归属走注册表（见 `site_id_of_label`）。
        let active_id =
            site_id_of_label(&active_label).filter(|id| sites.iter().any(|s| &s.id == id));
        let checked = active_id
            .as_ref()
            .and_then(|id| sites.iter().find(|s| &s.id == id))
            .map(|s| s.frameless)
            .unwrap_or(false);
        let frameless_item = CheckMenuItemBuilder::with_id("frameless", "无边框窗口（当前环境）")
            .checked(checked)
            .enabled(active_id.is_some())
            .build(app)
            .map_err(|e| e.to_string())?;
        builder = builder.item(&frameless_item);

        // 「新建窗口」：为当前激活的站点再开一个窗口（与已有窗口共享同一份登录态）。
        let newwin_item = MenuItemBuilder::with_id("newwindow", "新建窗口（当前环境）")
            .enabled(active_id.is_some())
            .build(app)
            .map_err(|e| e.to_string())?;
        builder = builder.item(&newwin_item);

        // 「彻底关闭」：销毁当前激活的站点窗口（不是隐藏到托盘）。
        let closewin_item = MenuItemBuilder::with_id("closewindow", "彻底关闭当前窗口（当前环境）")
            .enabled(active_id.is_some())
            .build(app)
            .map_err(|e| e.to_string())?;
        builder = builder.item(&closewin_item);

        builder = builder.item(&item("reload", "重新加载当前页面")?);
        builder = builder.item(&item("showall", "显示全部窗口")?);
    }
    builder = builder.separator();
    builder = builder.item(&item("quit", "退出")?);
    builder.build().map_err(|e| e.to_string())
}

// ---------------- 前端命令 ----------------

/// 返回当前配置（含配置文件路径），供设置页渲染。
#[tauri::command]
fn get_config(app: tauri::AppHandle) -> serde_json::Value {
    let cfg = cfg_state().lock().unwrap().clone();
    serde_json::json!({
        "sites": cfg.sites,
        "last_site_id": cfg.last_site_id,
        "ui_theme": cfg.ui_theme,
        "render_mode": cfg.render_mode,
        "dev_tools": cfg.dev_tools,
        // 设置页据此决定要不要显示「Linux 渲染」区块。
        "platform": std::env::consts::OS,
        "config_path": config::config_path(&app).map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "version": app.package_info().version.to_string(),
    })
}

/// 保存配置：校验 → 落盘 → 更新内存 → 刷新托盘 → 同步已打开窗口的外观与地址。
#[tauri::command]
fn save_config(app: tauri::AppHandle, sites: Vec<Site>) -> Result<Vec<Site>, String> {
    let (prev_last, prev_theme, prev_render, prev_dev_tools, prev_sites) = {
        let g = cfg_state().lock().unwrap();
        (
            g.last_site_id.clone(),
            g.ui_theme.clone(),
            g.render_mode.clone(),
            g.dev_tools,
            g.sites.clone(),
        )
    };
    let mut cfg = AppConfig {
        sites,
        last_site_id: prev_last,
        ui_theme: prev_theme,
        render_mode: prev_render,
        dev_tools: prev_dev_tools,
    };
    config::normalize(&mut cfg)?;
    config::save(&app, &cfg)?;

    {
        let mut state = cfg_state().lock().unwrap();
        *state = cfg.clone();
    }
    refresh_tray(&app)?;

    // 若某个已打开窗口的无边框设置变了，立即生效。
    for site in &cfg.sites {
        apply_frameless(&app, site);
    }

    let alive: Vec<String> = cfg.sites.iter().map(|s| s.id.clone()).collect();

    // 站点被删除 —— 把它名下还在跑的窗口一并销毁。
    //
    // 不这么做的话，这些窗口会变成「孤儿」：窗口归属是靠 WINDOW_SITES 注册表查的，但站点归属
    // 的兜底路径（`site_id_of_label`）里有一条是拿 label 去**配置里**找站点，站点一删这条就断了，
    // 于是托盘的「彻底关闭当前窗口 / 新建窗口 / 无边框」对该窗口全部置灰 ——
    // **这个窗口再也没有途径被彻底关闭**（只能 × 隐藏到托盘，WebView 永不释放）。
    // 设置页的「N 个窗口」计数同样看不到它。
    for old in &prev_sites {
        if alive.iter().any(|id| id == &old.id) {
            continue;
        }
        for label in site_window_labels(&app, &old.id) {
            log_line(&app, &format!("站点已删除，销毁孤儿窗口 {label}"));
            destroy_site_window(&app, &label);
        }
    }

    // 站点地址改了 —— 让它已开的窗口跟着走，否则窗口里还是旧地址，
    // 用户会以为「改了没生效」（只有下次全新打开才会用到新地址）。
    for site in &cfg.sites {
        let changed = prev_sites
            .iter()
            .find(|o| o.id == site.id)
            .map(|o| o.url != site.url)
            .unwrap_or(false);
        if !changed {
            continue;
        }
        let Ok(target) = url::Url::parse(&site.url) else {
            continue;
        };
        for label in site_window_labels(&app, &site.id) {
            if let Some(w) = app.get_webview_window(&label) {
                log_line(&app, &format!("地址已更新，重新导航 {label} -> {target}"));
                let _ = w.navigate(target.clone());
            }
        }
    }

    // 站点被删除后，别在凭据文件里留下孤儿记录。
    if let Err(e) = credentials::prune(&app, &alive) {
        eprintln!("[SAS PWA 客户端] 清理凭据失败：{e}");
    }

    Ok(cfg.sites)
}

/// 保存 UI 主题偏好（system / light / dark），不影响站点配置。
#[tauri::command]
fn set_ui_theme(app: tauri::AppHandle, theme: String) -> Result<(), String> {
    let mut cfg = cfg_state().lock().unwrap().clone();
    cfg.ui_theme = theme;
    config::save(&app, &cfg)?;
    *cfg_state().lock().unwrap() = cfg;
    Ok(())
}

/// 保存是否允许在窗口里打开开发者工具（F12 / 右键「检查」）。
///
/// **默认关闭**，而且只影响**之后新建的窗口**（wry 的 devtools 只能在 WebView 初始化时决定，
/// 已经跑起来的窗口改不了）。为什么默认关：站点窗口承载的是远程 SAS 页面，
/// DevTools 一旦可用，能碰到这个窗口的人就能读到注入脚本里用于自动填充的登录凭据。
/// 排查完页面问题建议关回去。
#[tauri::command]
fn set_dev_tools(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let mut cfg = cfg_state().lock().unwrap().clone();
    cfg.dev_tools = enabled;
    config::save(&app, &cfg)?;
    *cfg_state().lock().unwrap() = cfg;
    log_line(&app, &format!("dev tools saved: {enabled}（对新打开的窗口生效）"));
    Ok(())
}

/// 用系统默认浏览器打开链接（设置页「检查更新」跳到 Release 页用）。
///
/// 两道限制，避免这条命令变成「打开任意网址」的跳板：
///   1) 只接受 http/https，且主机名必须在下面这份白名单里（含子域）；
///   2) **只授权给本地设置窗口** —— remote-sites.json 里没有 `allow-open-url`，
///      远程 SAS 页面调不到它。
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    let parsed = url::Url::parse(url.trim()).map_err(|e| format!("链接无效：{e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("只支持 http/https 链接（收到 {other}）")),
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    const ALLOWED_DOMAINS: [&str; 2] = ["github.com", "githubusercontent.com"];
    let allowed = ALLOWED_DOMAINS
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")));
    if !allowed {
        return Err(format!("不允许打开该域名：{host}"));
    }

    let url = parsed.to_string();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// 别让 cmd 闪一个黑色控制台窗口。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // `start "" <url>`：那个空的标题参数是必须的，否则 start 会把 URL 当成窗口标题。
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("打开浏览器失败：{e}"))?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开浏览器失败（需要 xdg-open）：{e}"))?;
    }
    Ok(())
}

/// 保存 Linux 渲染模式（auto / smooth / compat）。
///
/// 实质改的是 WebKitGTK 的 DMA-BUF 开关（见 `tune_linux_webkit`），必须在 WebKitGTK
/// 初始化之前设置 —— 所以这里只落盘，**下次启动才生效**，返回归一化后的值给设置页回显。
#[tauri::command]
fn set_render_mode(app: tauri::AppHandle, mode: String) -> Result<String, String> {
    let mode = config::normalize_render_mode(&mode);
    let mut cfg = cfg_state().lock().unwrap().clone();
    cfg.render_mode = mode.clone();
    config::save(&app, &cfg)?;
    *cfg_state().lock().unwrap() = cfg;
    log_line(&app, &format!("render mode saved: {mode}（重启后生效）"));
    Ok(mode)
}

/// 打开指定站点；不传 id 时按「默认登录环境 → 最近打开 → 第一个」的顺序挑一个。
///
/// 必须是 async：同步命令运行在主线程的 IPC 回调栈里，从那里创建窗口会死锁，
/// 详见文件上方【关键规则】。
#[tauri::command]
async fn open_site(app: tauri::AppHandle, id: Option<String>) -> Result<(), String> {
    let id = id.or_else(|| {
        let cfg = cfg_state().lock().unwrap();
        cfg.sites
            .iter()
            .find(|s| s.default)
            .map(|s| s.id.clone())
            .or_else(|| cfg.last_site_id.clone())
            .or_else(|| cfg.sites.first().map(|s| s.id.clone()))
    });
    match id {
        Some(id) => open_site_window(&app, &id),
        None => Err("还没有配置任何 SAS 站点".to_string()),
    }
}

/// 设置页「新窗口」：为同一站点再开一个窗口（不聚焦已有窗口）。
///
/// 窗口之间共享同一份 WebView 数据目录 → cookie / SSO 会话同一份，新窗口即当前登录身份。
/// 必须 async：理由同 `open_site`（创建窗口不能在主线程 IPC 回调栈里发起）。
#[tauri::command]
async fn new_site_window(app: tauri::AppHandle, id: String) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("请先保存站点配置，再新开窗口".to_string());
    }
    create_new_site_window(&app, &id)
}

/// 站点窗口自绘标题条的「✕」：彻底关闭当前窗口（销毁 WebView，不是隐藏到托盘）。
///
/// 走自定义命令而不是 JS 的 `close()`：`close()` 会派发 `CloseRequested`，被上面的
/// 「关闭即隐藏」拦下就永远关不掉了。
#[tauri::command]
async fn close_window(app: tauri::AppHandle, window: tauri::Window) -> Result<(), String> {
    destroy_site_window(&app, window.label());
    Ok(())
}

/// 设置页站点行的「关闭窗口」：关掉该站点的全部窗口（销毁，不是隐藏）。
#[tauri::command]
async fn close_site_windows(app: tauri::AppHandle, site_id: String) -> Result<usize, String> {
    let labels = site_window_labels(&app, &site_id);
    let count = labels.len();
    for label in labels {
        destroy_site_window(&app, &label);
    }
    log_line(&app, &format!("close site windows: site={site_id} count={count}"));
    Ok(count)
}

/// 设置页：各站点当前已开的窗口数（显示「N 个窗口」，并决定「关闭窗口」是否可点）。
#[tauri::command]
async fn site_window_counts(app: tauri::AppHandle) -> Result<HashMap<String, usize>, String> {
    let sites = cfg_state().lock().unwrap().sites.clone();
    let mut counts = HashMap::new();
    for site in sites {
        let n = site_window_labels(&app, &site.id).len();
        if n > 0 {
            counts.insert(site.id, n);
        }
    }
    Ok(counts)
}

/// 设置页用：关闭自身（实际是隐藏，保留状态）。
#[tauri::command]
fn hide_settings(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = w.hide();
    }
}

/// 设置某个站点的无边框状态：更新内存 → 落盘 → 立即应用到窗口 → 刷新托盘勾选。
fn set_frameless(app: &tauri::AppHandle, site_id: &str, frameless: bool) -> Result<(), String> {
    {
        let mut state = cfg_state().lock().unwrap();
        let site = state
            .sites
            .iter_mut()
            .find(|s| s.id == site_id)
            .ok_or_else(|| "配置里找不到这个站点".to_string())?;
        site.frameless = frameless;
    }

    let cfg = cfg_state().lock().unwrap().clone();
    config::save(app, &cfg)?;

    if let Some(site) = get_site(site_id) {
        apply_frameless(app, &site);
    }
    let _ = refresh_tray(app);
    Ok(())
}

/// 切换某个站点窗口的无边框外观（去 / 回系统标题栏）。
///
/// 三个入口共用：站点窗口的 F11（原生钩子，见 `hotkey` 模块）、注入脚本里的 F11 兜底、
/// 以及设置页 / 托盘菜单。带 400ms 去重，避免一次按键被处理两次（等于没切换）。
fn toggle_frameless_for(app: &tauri::AppHandle, window_label: &str) -> Result<bool, String> {
    // 多开时标签可能带序号，站点归属走注册表而不是字符串裁剪。
    let id = site_id_of_label(window_label)
        .ok_or_else(|| "只有 SAS 站点窗口才能切换无边框".to_string())?;

    let current = get_site(&id)
        .map(|s| s.frameless)
        .ok_or_else(|| "配置里找不到这个站点".to_string())?;

    {
        let mut last = LAST_TOGGLE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap();
        let now = Instant::now();
        if let Some((label, at)) = last.as_ref() {
            if label == window_label && now.duration_since(*at) < TOGGLE_DEBOUNCE {
                return Ok(current);
            }
        }
        *last = Some((window_label.to_string(), now));
    }

    set_frameless(app, &id, !current)?;
    log_line(app, &format!("toggle frameless {id} -> {}", !current));
    Ok(!current)
}

/// 原生 F11 钩子的落地动作（钩子回调只负责投递，实际切换在这里执行）。
fn hotkey_toggle(app: &tauri::AppHandle, window_label: &str) {
    if let Err(e) = toggle_frameless_for(app, window_label) {
        eprintln!("[SAS PWA 客户端] F11 切换无边框失败：{e}");
    }
}

/// 切换无边框外观（去 / 回系统标题栏）：站点窗口按 F11，或托盘菜单「无边框窗口」都能触发。
///
/// 做成 async 命令：设置页之外没有 UI 可点，快捷键是唯一入口；在线程池里改 decorations
/// 不会阻塞主线程，也不会像同步命令那样落在 WebView 回调栈里。
#[tauri::command]
async fn toggle_frameless(app: tauri::AppHandle, window: tauri::Window) -> Result<bool, String> {
    toggle_frameless_for(&app, window.label())
}

/// 站点窗口自绘标题条的「⚙」：打开设置窗口（走后台线程创建，理由见上方【关键规则】）。
#[tauri::command]
async fn show_settings(app: tauri::AppHandle) -> Result<(), String> {
    open_settings_from_event(&app);
    Ok(())
}

/// 设置页：读取某站点已保存的登录凭据（只回用户名 / 是否已存密码，绝不下发密码）。
#[tauri::command]
fn get_credential(app: tauri::AppHandle, site_id: String) -> Option<credentials::CredentialView> {
    credentials::view(&app, &site_id)
}

/// 设置页：保存登录凭据（密码用 Windows DPAPI 加密后存盘）。
///
/// `password` 省略或为空 = 不修改已保存的密码；`username` 留空 = 清除该站点凭据。
#[tauri::command]
fn save_credential(
    app: tauri::AppHandle,
    site_id: String,
    username: String,
    password: Option<String>,
    auto_login: Option<bool>,
) -> Result<credentials::CredentialView, String> {
    if site_id.trim().is_empty() {
        return Err("请先保存站点配置，再保存登录凭据".to_string());
    }
    let view = credentials::upsert(
        &app,
        &site_id,
        &username,
        password.as_deref(),
        auto_login.unwrap_or(false),
    )?;
    log_line(
        &app,
        &format!(
            "credential saved: site={} user={} has_password={} auto_login={}",
            site_id, view.username, view.has_password, view.auto_login
        ),
    );
    Ok(view)
}

/// 设置页：清除某站点已保存的登录凭据。
#[tauri::command]
fn clear_credential(app: tauri::AppHandle, site_id: String) -> Result<(), String> {
    credentials::remove(&app, &site_id)?;
    log_line(&app, &format!("credential cleared: site={site_id}"));
    Ok(())
}

// ---------------- 命令行开关 ----------------
//
// 为什么需要：Linux 上的托盘图标依赖 StatusNotifier 宿主，部分桌面环境（WSLg、精简的 X11
// 会话等）根本没有 —— 托盘看不见，就意味着既没办法「收进托盘」，也没办法退出（托盘的「退出」
// 是唯一入口），只能去杀进程。给命令行开关补上这两个缺口；Windows 上也能用来写脚本 / 快捷方式。
//
//   --quit / --exit / -q   退出正在运行的实例
//   --hide                 把全部窗口收起来（等同于逐个按「隐藏到托盘」）
//
// 两个时机都要认：
//   * 已经有一个实例在跑 → 新进程本身不建窗口，只把动作转达过去（单实例插件的回调里处理）；
//   * 自己是第一个实例   → `--quit` 直接退出（既然目标就是「不要有实例在跑」），
//                          `--hide` 没有窗口可藏，按正常启动走。

/// 命令行里能识别的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliAction {
    Quit,
    Hide,
}

/// 从参数表解析动作；`--quit` 优先于 `--hide`（argv[0] 是程序路径，会自然落进 `_` 分支）。
fn cli_action<S: AsRef<str>>(args: &[S]) -> Option<CliAction> {
    let mut hide = false;
    for arg in args {
        match arg.as_ref() {
            "--quit" | "--exit" | "-q" => return Some(CliAction::Quit),
            "--hide" => hide = true,
            _ => {}
        }
    }
    hide.then_some(CliAction::Hide)
}

/// 执行命令行动作。
fn run_cli_action(app: &tauri::AppHandle, action: CliAction) {
    match action {
        CliAction::Quit => {
            log_line(app, "cli: quit");
            app.exit(0);
        }
        CliAction::Hide => {
            // 连设置窗口一起收起来：用户的意图是「把客户端收起来」，留一个设置窗在外面更费解。
            let labels: Vec<String> = app.webview_windows().keys().cloned().collect();
            for label in labels {
                if let Some(w) = app.get_webview_window(&label) {
                    let _ = w.hide();
                }
            }
            log_line(app, "cli: hide");
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Linux（WebKitGTK）适配必须在 Tauri 初始化之前完成（DMA-BUF 开关、中文字体检查）。
    // 渲染模式来自配置文件，此时还没有 AppHandle，只能直接读一遍 config.json。
    #[cfg(target_os = "linux")]
    let linux_notes = tune_linux_webkit(&config::pre_init_render_mode());
    #[cfg(not(target_os = "linux"))]
    let linux_notes = String::new();

    let mut builder = tauri::Builder::default();

    // 单实例锁：避免重复启动；再次启动时聚焦最近使用的窗口。
    builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        // 命令行动作优先：`sas-pwa-client --quit` 的意图是退出，不是把窗口聚焦到眼前。
        if let Some(action) = cli_action(&argv) {
            run_cli_action(app, action);
            return;
        }
        // 重复启动只做聚焦：有活动窗口就切过去，否则打开设置窗口（不自动创建站点窗口）。
        let label = active_window().lock().unwrap().clone();
        if label.is_empty() {
            open_settings_from_event(app);
        } else {
            focus_window(app, &label);
        }
    }));

    // 窗口位置 / 尺寸记忆（按窗口 label 记录：同一站点的第 2、3 个窗口各记各自的）。
    // 刻意**不含** VISIBLE / DECORATIONS / FULLSCREEN：
    //   * 我们的窗口是「隐藏创建 → 页面加载完再 show」，恢复可见性会跟「关闭 = 隐藏到托盘」打架
    //     （插件默认 flags 是 all()，包含 VISIBLE，会把窗口恢复成隐藏/显示而不是我们要的状态）；
    //   * 无边框由站点配置决定，不该由窗口状态恢复。
    builder = builder.plugin(
        tauri_plugin_window_state::Builder::new()
            .with_state_flags(
                tauri_plugin_window_state::StateFlags::POSITION
                    | tauri_plugin_window_state::StateFlags::SIZE
                    | tauri_plugin_window_state::StateFlags::MAXIMIZED,
            )
            .build(),
    );

    builder
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            open_site,
            new_site_window,
            close_window,
            close_site_windows,
            site_window_counts,
            hide_settings,
            set_ui_theme,
            set_render_mode,
            set_dev_tools,
            open_url,
            toggle_frameless,
            show_settings,
            get_credential,
            save_credential,
            clear_credential
        ])
        .setup(move |app| {
            // 先载入持久化配置，再初始化内存状态。
            let loaded = config::load(app.handle());
            let _ = CONFIG.set(Mutex::new(loaded.clone()));
            log_line(
                app.handle(),
                &format!(
                    "start: dev={} version={} sites={}",
                    tauri::is_dev(),
                    app.package_info().version,
                    loaded.sites.len()
                ),
            );

            if !linux_notes.is_empty() {
                log_line(app.handle(), &format!("linux: {linux_notes}"));
            }

            // 命令行开关（见文件上方说明）：走到这里说明当前进程就是第一个实例，没有别的实例
            // 可以转达。`--quit` 的意图是「不要有实例在跑」，那就直接退出、不建任何窗口；
            // `--hide` 此时没有窗口可藏，按正常启动处理即可。
            if cli_action(&std::env::args().collect::<Vec<String>>()) == Some(CliAction::Quit) {
                run_cli_action(app.handle(), CliAction::Quit);
                return Ok(());
            }

            let _ = build_tray_at_startup(app.handle());

            // 站点窗口的 F11：优先交给原生低级键盘钩子（WebView2 会把 F11 当浏览器
            // 加速键吃掉，页面里的 keydown 不一定收得到）。装不上时工具栏里的注入脚本兜底。
            // 必须在这里（窗口创建之前）安装：钩子回调在主线程的消息泵里执行。
            let hooked = hotkey::install(app.handle().clone(), hotkey_toggle);
            log_line(app.handle(), &format!("F11 native hook: {hooked}"));

            // 启动策略（默认环境在设置页里指定）：
            //   配了「默认登录环境」→ 直接连接该环境；
            //   没配               → 打开设置窗口，由用户选择要登录的服务器。
            // 注意：站点窗口不能在 setup 里就地创建（事件循环还没开始派发消息会卡死），
            // 交给后台线程发起，创建请求入队、等事件循环跑起来后执行。
            let default_id = loaded.sites.iter().find(|s| s.default).map(|s| s.id.clone());
            match default_id {
                Some(id) => {
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        if let Err(e) = open_site_window(&handle, &id) {
                            eprintln!("[SAS PWA 客户端] 打开默认站点失败：{e}");
                            log_line(&handle, &format!("open default site failed: {e}"));
                            open_settings_from_event(&handle);
                        }
                    });
                }
                None => open_settings(app.handle()),
            }

            Ok(())
        })
        .on_menu_event(|app, event: MenuEvent| {
            let id = event.id().as_ref().to_string();
            if let Some(site_id) = id.strip_prefix("open:") {
                // 与 open_site 命令同理：创建窗口必须在主线程之外发起，避免死锁。
                let app = app.clone();
                let site_id = site_id.to_string();
                std::thread::spawn(move || {
                    if let Err(e) = open_site_window(&app, &site_id) {
                        eprintln!("[SAS PWA 客户端] 打开站点失败：{e}");
                    }
                });
                return;
            }
            match id.as_str() {
                "settings" => open_settings_from_event(app),
                "frameless" => {
                    // 托盘里的「无边框窗口」：作用于当前激活的站点窗口。
                    let label = active_window().lock().unwrap().clone();
                    if let Some(site_id) = site_id_of_label(&label) {
                        let app = app.clone();
                        std::thread::spawn(move || {
                            if let Some(next) = get_site(&site_id).map(|s| !s.frameless) {
                                if let Err(e) = set_frameless(&app, &site_id, next) {
                                    eprintln!("[SAS PWA 客户端] 切换无边框失败：{e}");
                                }
                            }
                        });
                    }
                }
                "newwindow" => {
                    // 托盘里的「新建窗口」：为当前激活的站点再开一个窗口（共享同一份登录态）。
                    // 与 open: 一样走后台线程创建（同步创建会死锁，见文件顶部说明）。
                    let label = active_window().lock().unwrap().clone();
                    if let Some(site_id) = site_id_of_label(&label) {
                        let app = app.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = create_new_site_window(&app, &site_id) {
                                eprintln!("[SAS PWA 客户端] 新建窗口失败：{e}");
                                log_line(&app, &format!("new window failed: {e}"));
                            }
                        });
                    }
                }
                "closewindow" => {
                    // 托盘里的「彻底关闭」：销毁当前激活的站点窗口（后台线程里做，不阻塞回调）。
                    let label = active_window().lock().unwrap().clone();
                    if !label.is_empty() {
                        let app = app.clone();
                        std::thread::spawn(move || destroy_site_window(&app, &label));
                    }
                }
                "reload" => {
                    let label = active_window().lock().unwrap().clone();
                    if let Some(w) = app.get_webview_window(&label) {
                        let _ = w.eval("location.reload();");
                    }
                }
                "showall" => {
                    for (label, w) in app.webview_windows() {
                        let _ = w.show();
                        let _ = w.unminimize();
                        if label != SETTINGS_LABEL {
                            let _ = w.set_focus();
                        }
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // 关闭一律「收进托盘」：页面继续跑、保活脉冲继续，会话不会被服务端按空闲超时回收。
                    // 要真正释放窗口（销毁 WebView、回收内存）走这三个入口：
                    // 自绘标题条的 ✕、托盘「彻底关闭当前窗口（当前环境）」、设置页站点行的「关闭窗口」。
                    api.prevent_close();
                    let _ = window.hide();
                }
                // 窗口真被销毁（退出前 / 主动 destroy）时清掉注册表，别留着孤儿标签。
                tauri::WindowEvent::Destroyed => {
                    // 收尾统一放这里（`destroy()` 与「放行的关闭」都会走到）：先取站点 id 再清注册表，
                    // 之后才查得到「同站点还剩哪些窗口」用来接管「当前窗口」。
                    let label = window.label().to_string();
                    let site_id = site_id_of_label(&label);
                    window_sites().lock().unwrap().remove(&label);
                    log_line(window.app_handle(), &format!("destroyed {label}"));
                    // 不要在这个回调里碰窗口表 / 托盘（可能正握着管理器锁）：丢后台线程做。
                    let app = window.app_handle().clone();
                    std::thread::spawn(move || {
                        let mut active = active_window().lock().unwrap();
                        if *active == label {
                            // 关掉的正是当前窗口 → 交给同站点其它窗口；一个都不剩就清空
                            // （托盘左键会回落到「打开设置页」）。
                            *active = site_id
                                .as_deref()
                                .and_then(|id| site_window_labels(&app, id).into_iter().next())
                                .unwrap_or_default();
                        }
                        drop(active);
                        let _ = refresh_tray(&app);
                    });
                }
                // 记住最后一个获得焦点的站点窗口：托盘左键显隐、菜单「无边框窗口」都以它为准。
                tauri::WindowEvent::Focused(true) => {
                    let label = window.label().to_string();
                    if label.starts_with(WINDOW_PREFIX) {
                        let changed = {
                            let mut active = active_window().lock().unwrap();
                            if *active == label {
                                false
                            } else {
                                *active = label.clone();
                                true
                            }
                        };
                        // 勾选状态跟着切，免得下次打开托盘菜单看到旧状态。
                        if changed {
                            let app = window.app_handle().clone();
                            std::thread::spawn(move || {
                                let _ = refresh_tray(&app);
                            });
                        }
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("运行 SAS PWA 客户端失败");
}

/// 构建托盘图标（菜单内容由 refresh_tray 动态替换）。
fn build_tray_at_startup(app: &tauri::AppHandle) -> Result<(), String> {
    let menu = build_tray_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("SAS")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let label = active_window().lock().unwrap().clone();
                let visible = app
                    .get_webview_window(&label)
                    .and_then(|w| w.is_visible().ok())
                    .unwrap_or(false);
                if visible {
                    if let Some(w) = app.get_webview_window(&label) {
                        let _ = w.hide();
                    }
                } else if !label.is_empty() {
                    focus_window(app, &label);
                } else {
                    open_settings_from_event(app);
                }
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;
    Ok(())
}
