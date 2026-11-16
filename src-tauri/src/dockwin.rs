//! 悬浮窗口栏：独立的置顶透明窗口（label = `dock`），页面是本地的 `dock.html`。
//!
//! 为什么不再注入到站点页面：站点页面的可视区域从系统标题栏下方开始，胶囊画在页面里
//! 就到不了屏幕顶端。独立窗口不受这个限制，可以拖到屏幕任何位置。
//!
//! 职责划分：
//!   * 页面（src/dock.js）：渲染、点击、拖动起手（调用系统拖动）、外观面板；
//!   * 本模块：窗口尺寸、位置、拖动结束后的吸边、贴边隐藏与唤出、有无站点窗口时显隐。
//!
//! 位置偏好仍存在 dock.json（DockPrefs）。自由位置用比例 cx / cy 表示，吸附位置用 edge 表示。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "dock";

/// 窗口四周的留白（逻辑像素），给阴影用。必须与 src/dock.css 的布局一致。
const MARGIN: f64 = 14.0;
/// 贴边隐藏时露出的那一条（逻辑像素）。鼠标碰到这一条就会唤出。
const TUCK: f64 = 6.0;
/// 拖到离屏幕边缘这么近（逻辑像素）就吸附过去。
const SNAP: f64 = 28.0;
/// 程序自己摆放窗口时，短时间内忽略 Moved 事件，避免把程序的移动当成用户拖动。
const SUPPRESS_MS: u64 = 400;

/// 胶囊是否展开。收起时才会贴边隐藏。
static OPEN: AtomicBool = AtomicBool::new(false);
static SUPPRESS_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
static LAST_MOVE_MS: AtomicU64 = AtomicU64::new(0);
/// 已经有一个等待拖动结束的线程，避免每个 Moved 事件都起一个线程。
static PENDING: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 钉住时不贴边隐藏。
fn pinned_now() -> bool {
    crate::DOCK_PINNED.load(Ordering::SeqCst)
}

/// 创建悬浮窗口（隐藏着创建，有站点窗口时由 `sync` 显示）。
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("dock.html".into()))
        .title("Windows")
        // 【必须】和站点窗口 / 设置窗口用同一组 WebView2 启动参数：共用同一个用户数据目录的
        // WebView2 环境参数必须完全一致，否则这个窗口在事件循环里创建失败（build() 却返回 Ok，
        // 之后对它的任何查询都是 FailedToReceiveMessage）。
        .additional_browser_args(crate::BROWSER_ARGS)
        .inner_size(240.0, 80.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .visible(false)
        .build()?;
    place(app, &w, true);
    Ok(())
}

/// 有站点窗口可见时显示悬浮栏，全部隐藏到托盘时一起隐藏。
pub fn sync(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else {
        return;
    };
    let any_site_visible = app.webview_windows().iter().any(|(label, win)| {
        label.starts_with(crate::WINDOW_PREFIX) && win.is_visible().unwrap_or(false)
    });
    if any_site_visible {
        if !w.is_visible().unwrap_or(false) {
            // 先把悬浮窗口挪到当前站点窗口所在的显示器，再按偏好精确摆放。
            follow_active_monitor(app, &w);
            place(app, &w, !OPEN.load(Ordering::SeqCst));
            // 跨显示器（DPI 不同）后窗口尺寸 / 缩放系数要等系统处理完才稳定，稍后再摆一次。
            {
                let h = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(350));
                    if let Some(w) = h.get_webview_window(LABEL) {
                        place(&h, &w, !OPEN.load(Ordering::SeqCst));
                    }
                });
            }
            let shown = w.show();
            crate::log_line(
                app,
                &format!(
                    "dock show: result={:?} visible={:?} pos={:?} size={:?} monitor={:?}",
                    shown.map_err(|e| e.to_string()),
                    w.is_visible(),
                    w.outer_position().map(|p| (p.x, p.y)),
                    w.outer_size().map(|s| (s.width, s.height)),
                    w.current_monitor().map(|m| m.map(|m| {
                        let r = m.work_area();
                        (r.position.x, r.position.y, r.size.width, r.size.height)
                    })),
                ),
            );
        }
    } else {
        let _ = w.hide();
    }
}

/// 页面（Ctrl+` 兜底）或原生钩子请求切换悬浮栏显隐。
#[tauri::command]
pub fn toggle_dock(app: AppHandle) {
    toggle(&app);
}

/// 快捷键 Ctrl+`：让悬浮栏切换展开 / 收起。悬浮栏此时必然可见（触发它的是站点窗口）。
pub fn toggle(app: &AppHandle) {
    let _ = app.emit_to(LABEL, "dock-toggle", ());
}


/// 把悬浮窗口先挪到「当前激活的站点窗口」所在的显示器（只换显示器，精确位置随后由 `place` 算）。
/// 多显示器下站点窗口常在副屏，而新建的悬浮窗口默认落在主屏，用户会以为胶囊没出现。
fn follow_active_monitor(app: &AppHandle, w: &tauri::WebviewWindow) {
    let label = crate::active_window().lock().unwrap().clone();
    let Some(site) = app.get_webview_window(&label) else {
        return;
    };
    let Ok(Some(target)) = site.current_monitor() else {
        return;
    };
    if let Ok(Some(cur)) = w.current_monitor() {
        if cur.position() == target.position() {
            return;
        }
    }
    let r = target.work_area();
    SUPPRESS_UNTIL_MS.store(now_ms() + SUPPRESS_MS, Ordering::SeqCst);
    let _ = w.set_position(PhysicalPosition::new(r.position.x + 40, r.position.y + 40));
}

/// 所在显示器的工作区（去掉任务栏）：(x, y, 宽, 高)，物理像素。
fn work_area(w: &tauri::WebviewWindow) -> Option<(i32, i32, i32, i32)> {
    let monitor = w
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| w.primary_monitor().ok().flatten())?;
    let r = monitor.work_area();
    Some((r.position.x, r.position.y, r.size.width as i32, r.size.height as i32))
}

/// 按偏好计算窗口位置并移动过去。`tuck` 为真且开启了贴边隐藏时，窗口滑到屏幕外只留一条。
fn place(app: &AppHandle, w: &tauri::WebviewWindow, tuck: bool) {
    let Some((ax, ay, aw, ah)) = work_area(w) else {
        return;
    };
    let Ok(size) = w.outer_size() else {
        return;
    };
    let s = w.scale_factor().unwrap_or(1.0);
    let (ww, wh) = (size.width as i32, size.height as i32);
    let m = (MARGIN * s).round() as i32;
    let tk = (TUCK * s).round() as i32;
    let prefs = crate::dock_prefs(app).lock().unwrap().clone();

    // 自由位置：比例 → 像素。范围按「内容区」算，内容可以正好贴到屏幕边。
    let dx = (aw - ww + 2 * m).max(1) as f64;
    let dy = (ah - wh + 2 * m).max(1) as f64;
    let fx = ax - m + (prefs.cx.clamp(0.0, 1.0) * dx).round() as i32;
    let fy = ay - m + (prefs.cy.clamp(0.0, 1.0) * dy).round() as i32;

    let (mut x, mut y) = match prefs.edge.as_str() {
        "top" => (fx, ay - m),
        "bottom" => (fx, ay + ah - wh + m),
        "left" => (ax - m, fy),
        "right" => (ax + aw - ww + m, fy),
        _ => (fx, fy),
    };

    if tuck && !pinned_now() && !prefs.edge.is_empty() && prefs.edge_hide {
        // 窗口四周有 m 的透明留白（阴影用）。要让「内容」露出 tk，窗口得多挪出 m，
        // 否则露出来的全是透明留白，肉眼看不到，鼠标也够不着胶囊本身。
        match prefs.edge.as_str() {
            "top" => y = ay + tk + m - wh,
            "bottom" => y = ay + ah - tk - m,
            "left" => x = ax + tk + m - ww,
            "right" => x = ax + aw - tk - m,
            _ => {}
        }
    } else {
        // 不隐藏时保证整个窗口仍在工作区内（窗口比屏幕还大的极端情况下以左上角为准）。
        let hi_x = (ax + aw - ww + m).max(ax - m);
        let hi_y = (ay + ah - wh + m).max(ay - m);
        x = x.max(ax - m).min(hi_x);
        y = y.max(ay - m).min(hi_y);
    }

    if let Ok(cur) = w.outer_position() {
        if cur.x == x && cur.y == y {
            return;
        }
    }
    SUPPRESS_UNTIL_MS.store(now_ms() + SUPPRESS_MS, Ordering::SeqCst);
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = app; // 保留参数：后续若要读其它偏好可直接用
}

/// 页面请求改变窗口尺寸（内容变化或弹出菜单/外观面板）。位置随之重算，保持贴边。
#[tauri::command]
pub fn dock_fit(app: AppHandle, width: f64, height: f64) {
    let Some(w) = app.get_webview_window(LABEL) else {
        return;
    };
    let _ = w.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
    place(&app, &w, !OPEN.load(Ordering::SeqCst));
}

/// 展开 / 收起。收起且开启贴边隐藏、并且贴着边时，窗口滑出屏幕只留一条。
#[tauri::command]
pub fn dock_set_open(app: AppHandle, open: bool) {
    OPEN.store(open, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window(LABEL) {
        place(&app, &w, !open);
    }
}

/// 窗口移动事件：用户拖动时会连续触发。停下来 150ms 后再判断是否吸附。
pub fn on_moved(app: &AppHandle) {
    if now_ms() < SUPPRESS_UNTIL_MS.load(Ordering::SeqCst) {
        return;
    }
    LAST_MOVE_MS.store(now_ms(), Ordering::SeqCst);
    if PENDING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(150));
            if now_ms().saturating_sub(LAST_MOVE_MS.load(Ordering::SeqCst)) >= 150 {
                break;
            }
        }
        PENDING.store(false, Ordering::SeqCst);
        finish_drag(&app);
    });
}

/// 拖动结束：如果离某条屏幕边很近就吸附过去，并把位置存进偏好。
fn finish_drag(app: &AppHandle) {
    if now_ms() < SUPPRESS_UNTIL_MS.load(Ordering::SeqCst) {
        return;
    }
    let Some(w) = app.get_webview_window(LABEL) else {
        return;
    };
    let Some((ax, ay, aw, ah)) = work_area(&w) else {
        return;
    };
    let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) else {
        return;
    };
    let s = w.scale_factor().unwrap_or(1.0);
    let (ww, wh) = (size.width as i32, size.height as i32);
    let m = MARGIN * s;
    let snap = SNAP * s;

    // 内容区（去掉四周留白）到四条屏幕边的距离。
    let cl = pos.x as f64 + m;
    let cr = pos.x as f64 + size.width as f64 - m;
    let ct = pos.y as f64 + m;
    let cb = pos.y as f64 + size.height as f64 - m;
    let candidates = [
        ("top", ct - ay as f64),
        ("bottom", (ay + ah) as f64 - cb),
        ("left", cl - ax as f64),
        ("right", (ax + aw) as f64 - cr),
    ];
    let edge = candidates
        .iter()
        .filter(|(_, d)| *d <= snap)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(e, _)| *e)
        .unwrap_or("");

    let dx = (aw - ww + 2 * m.round() as i32).max(1) as f64;
    let dy = (ah - wh + 2 * m.round() as i32).max(1) as f64;
    let m_i = m.round() as i32;
    let cx = (((pos.x - (ax - m_i)) as f64) / dx).clamp(0.0, 1.0);
    let cy = (((pos.y - (ay - m_i)) as f64) / dy).clamp(0.0, 1.0);

    let saved = {
        let mut g = crate::dock_prefs(app).lock().unwrap();
        g.edge = edge.to_string();
        g.cx = match edge {
            "left" => 0.0,
            "right" => 1.0,
            _ => cx,
        };
        g.cy = match edge {
            "top" => 0.0,
            "bottom" => 1.0,
            _ => cy,
        };
        g.clone()
    };
    let _ = crate::config::save_dock_prefs(app, &saved);

    // 按新的吸附结果摆放（收起时会直接滑到边上）。
    place(app, &w, !OPEN.load(Ordering::SeqCst));
    crate::emit_windows_changed(app);
}
