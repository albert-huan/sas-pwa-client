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
      bar.style.cssText = 'position:fixed;top:0;left:0;right:0;height:28px;z-index:2147483647;display:none;background:rgba(20,20,25,0.55);color:#fff;font:12px/28px system-ui,sans-serif;user-select:none;';
      var title = document.createElement('span');
      title.id = BAR_ID + '-title';
      title.setAttribute('data-tauri-drag-region', '');
      title.textContent = ' ' + (document.title || '');
      title.style.cssText = 'padding-left:10px;';
      var mk = function(label, fn){
        var b = document.createElement('button');
        b.textContent = label;
        b.style.cssText = 'float:right;height:28px;width:34px;border:0;background:transparent;color:#fff;cursor:pointer;font-size:13px;';
        b.addEventListener('click', fn);
        return b;
      };
      bar.appendChild(title);
      bar.appendChild(mk('×', function(){ if (window.__TAURI__ && window.__TAURI__.window) window.__TAURI__.window.getCurrentWindow().hide(); }));
      bar.appendChild(mk('–', function(){ if (window.__TAURI__ && window.__TAURI__.window) window.__TAURI__.window.getCurrentWindow().minimize(); }));
      bar.appendChild(mk('⚙', function(){ if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('show_settings'); }));
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

/// 诊断日志：追加到配置目录的 debug.log，便于远程排查（对正常使用无影响）。
fn log_line(app: &tauri::AppHandle, msg: &str) {
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("debug.log"))
        {
            let _ = std::io::Write::write_fmt(&mut f, format_args!("[{secs}] {msg}\n"));
        }
    }
}

/// 应用无边框外观（含自绘标题条显隐）。仅在需要改变时才动 decorations。
fn apply_frameless(app: &tauri::AppHandle, site: &Site) {
    if let Some(w) = app.get_webview_window(&window_label(&site.id)) {
        let decorated = w.is_decorated().unwrap_or(true);
        if decorated == site.frameless {
            let _ = w.set_decorations(!site.frameless);
        }
        let script = format!(
            "if(window.__sasShowFramelessBar)window.__sasShowFramelessBar({}, {});",
            site.frameless,
            serde_json::to_string(&site.name).unwrap_or_else(|_| "\"\"".into())
        );
        let _ = w.eval(&script);
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
fn create_site_window(app: &tauri::AppHandle, site: &Site) -> Result<(), String> {
    let label = window_label(&site.id);
    let target = url::Url::parse(&site.url).map_err(|e| format!("地址无效：{e}"))?;
    log_line(app, &format!("creating {label} -> {target}"));

    // 防白屏：窗口先隐藏创建，等页面真正加载完成（on_page_load Finished）再显示，
    // 否则 Windows/WebView2 常把「首帧空白」直接呈现给用户。
    let shown = Arc::new(AtomicBool::new(false));
    let shown_cb = shown.clone();
    let frameless = site.frameless;
    let site_name = site.name.clone();

    let built =
        WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::External(target.clone()))
            .title(site.name.clone())
            .initialization_script(render_script(app, site))
            .additional_browser_args(BROWSER_ARGS)
            .devtools(true)
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
    // 这里不 show，等页面加载完成再显示（见 on_page_load）；只先登记为当前窗口。
    {
        let mut active = active_window().lock().unwrap();
        *active = label.clone();
    }
    apply_frameless(app, site);

    // 兜底：
    //  1) 1.5s 后复查窗口实际地址，不对就重新导航一次（防止首次导航丢失导致白屏）；
    //  2) 再过 2s 无论页面有没有触发 Finished（证书错误 / 登录跳转卡住）都强制显示，
    //     否则窗口一直隐藏，看起来就像「点了打开没反应」。
    let handle = app.clone();
    let lbl = label.clone();
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
            let ok = url::Url::parse(&cur).map(|c| same_site(&c, &tgt)).unwrap_or(false);
            if ok {
                // 导航已经发生，直接显示，不等 Finished（有些站点一直挂着重定向不会 Finished）。
                if !shown_thread.swap(true, Ordering::SeqCst) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            } else {
                log_line(&handle, &format!("post-check mismatch -> navigate {tgt}"));
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

/// 打开（或聚焦）某个 SAS 站点窗口。
fn open_site_window(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let site = get_site(id).ok_or_else(|| format!("未找到站点：{id}"))?;
    let label = window_label(&site.id);
    let target = url::Url::parse(&site.url).map_err(|e| e.to_string())?;
    log_line(
        app,
        &format!("open id={} label={} url={}", site.id, label, target),
    );

    if let Some(w) = app.get_webview_window(&label) {
        // 地址被改过才重新导航，避免每次点击都整页刷新、丢掉会话状态。
        let stale = match w.url() {
            Ok(cur) => !same_site(&cur, &target),
            Err(_) => true,
        };
        if stale {
            let _ = w.navigate(target);
        }
        focus_window(app, &label);
    } else {
        // 直接创建：在命令线程调用时 tauri 会把创建请求派发到事件循环，安全。
        create_site_window(app, &site)?;
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

/// 打开设置窗口（本地 dist 页面）。
fn open_settings(app: &tauri::AppHandle) {
    if app.get_webview_window(SETTINGS_LABEL).is_some() {
        focus_window(app, SETTINGS_LABEL);
        return;
    }
    // 同样直接创建（不要走 run_on_main_thread，理由见文件顶部说明）。
    let res = (|| -> Result<(), String> {
        let built = WebviewWindowBuilder::new(
            app,
            SETTINGS_LABEL,
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("SAS 客户端 · 设置")
        .additional_browser_args(BROWSER_ARGS)
        .devtools(true)
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
        let active_id = active_label
            .strip_prefix(WINDOW_PREFIX)
            .map(|s| s.to_string())
            .filter(|id| sites.iter().any(|s| &s.id == id));
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
        "config_path": config::config_path(&app).map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "version": app.package_info().version.to_string(),
    })
}

/// 保存配置：校验 → 落盘 → 更新内存 → 刷新托盘 → 同步已打开窗口的外观。
#[tauri::command]
fn save_config(app: tauri::AppHandle, sites: Vec<Site>) -> Result<Vec<Site>, String> {
    let (prev_last, prev_theme) = {
        let g = cfg_state().lock().unwrap();
        (g.last_site_id.clone(), g.ui_theme.clone())
    };
    let mut cfg = AppConfig {
        sites,
        last_site_id: prev_last,
        ui_theme: prev_theme,
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

    // 站点被删除后，别在凭据文件里留下孤儿记录。
    let alive: Vec<String> = cfg.sites.iter().map(|s| s.id.clone()).collect();
    if let Err(e) = credentials::prune(&app, &alive) {
        eprintln!("[SAS PWA 客户端] 清理凭据失败：{e}");
    }

    Ok(cfg.sites)
}

/// 打开指定站点；不传 id 时按「默认登录环境 → 最近打开 → 第一个」的顺序挑一个。
///
/// 必须是 async：同步命令在主线程（IPC 回调栈）里执行，会触发窗口创建死锁，
/// 保存 UI 主题偏好（system / light / dark），不影响站点配置。
#[tauri::command]
fn set_ui_theme(app: tauri::AppHandle, theme: String) -> Result<(), String> {
    let mut cfg = cfg_state().lock().unwrap().clone();
    cfg.ui_theme = theme;
    config::save(&app, &cfg)?;
    *cfg_state().lock().unwrap() = cfg;
    Ok(())
}

/// 详见文件上半部分 `create_site_window` 上方的说明。
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
    let id = window_label
        .strip_prefix(WINDOW_PREFIX)
        .ok_or_else(|| "只有 SAS 站点窗口才能切换无边框".to_string())?
        .to_string();

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    // 单实例锁：避免重复启动；再次启动时聚焦最近使用的窗口。
    builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        // 重复启动只做聚焦：有活动窗口就切过去，否则打开设置窗口（不自动创建站点窗口）。
        let label = active_window().lock().unwrap().clone();
        if label.is_empty() {
            open_settings_from_event(app);
        } else {
            focus_window(app, &label);
        }
    }));

    builder
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            open_site,
            hide_settings,
            set_ui_theme,
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
                    if let Some(site_id) = label.strip_prefix(WINDOW_PREFIX).map(|s| s.to_string()) {
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
                    // 所有窗口关闭时都只隐藏到托盘，保持会话存活。
                    api.prevent_close();
                    let _ = window.hide();
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
