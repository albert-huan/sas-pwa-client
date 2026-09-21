//! WebView2 权限请求接管（仅 Windows）。
//!
//! Tauri/wry 只在开启剪贴板访问时处理 `CLIPBOARD_READ` 这一项，其余权限请求（网页通知、
//! 摄像头/麦克风、定位……）一律落到 WebView2 的默认行为 —— 所以网页通知这类 SAS 会用到的
//! 能力在客户端里往往是哑的（`Notification.requestPermission()` 直接失败、`new Notification()`
//! 不弹）。
//!
//! 这里从 `PlatformWebview` 拿 controller 手动挂 `PermissionRequested`：
//!   * SAS 确实需要的那几项（网页通知 / 剪贴板读取 / 多文件下载）**直接放行**，
//!     等价于 Edge 里用户点了「允许」并记住该站点；
//!   * 其余保持 WebView2 默认（一般是拒绝），但会写一条 debug.log，
//!     需要放开哪一项时按日志里的 kind 编号加白名单即可（改动只在这个文件）。
//!
//! 非 Windows（Linux = WebKitGTK）要用另一套 API（`WebKitWebView::permission-request` 信号，
//! 由 webkit2gtk-rs 暴露），这里先不处理，保持现状。

/// 给站点窗口装上权限请求处理器。失败只记日志（调用方处理），不影响窗口使用。
#[cfg(windows)]
pub fn install(window: &tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND as Kind, COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ,
        COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS,
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    };
    use webview2_com::PermissionRequestedEventHandler;

    let label = window.label().to_string();
    window
        .with_webview(move |pw| {
            let controller = pw.controller();
            // 回调跑在 WebView2 的 UI 线程上：只做「判定 + 记一行日志」，不做重活、不建窗口。
            let handler = PermissionRequestedEventHandler::create(Box::new(move |_sender, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = Kind::default();
                // webview2-com 0.38 把 EventArgs 上的方法都标成了 unsafe（COM 约定）。
                unsafe { args.PermissionKind(&mut kind)? };
                let allow = kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
                    || kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ
                    || kind == COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS;
                if allow {
                    unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)? };
                }
                crate::log_line(
                    &app,
                    &format!(
                        "permission {label}: {} ({}) -> {}",
                        kind_label(kind.0),
                        kind.0,
                        if allow { "allow" } else { "default" }
                    ),
                );
                Ok(())
            }));
            unsafe {
                let Ok(core) = controller.CoreWebView2() else {
                    return;
                };
                let mut token = 0i64;
                let _ = core.add_PermissionRequested(&handler, &mut token);
            }
        })
        .map_err(|e| format!("安装权限处理器失败：{e}"))
}

/// kind 编号 → 名字（见 COREWEBVIEW2_PERMISSION_KIND 枚举），只为日志好读。
#[cfg(windows)]
fn kind_label(kind: i32) -> &'static str {
    match kind {
        1 => "microphone",
        2 => "camera",
        3 => "geolocation",
        4 => "notifications",
        5 => "other-sensors",
        6 => "clipboard-read",
        7 => "multiple-downloads",
        8 => "file-read-write",
        9 => "autoplay",
        10 => "local-fonts",
        _ => "unknown",
    }
}

/// 非 Windows：WebKitGTK 走的是 `WebKitWebView::permission-request` 信号，见模块注释。
#[cfg(not(windows))]
pub fn install(_window: &tauri::WebviewWindow, _app: tauri::AppHandle) -> Result<(), String> {
    Ok(())
}
