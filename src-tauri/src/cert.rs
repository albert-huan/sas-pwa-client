//! WebView2 证书 / HSTS 相关处理（仅 Windows）。
//!
//! ## 证书错误放行（`ServerCertificateErrorDetected`）
//!
//! 公司内部 SAS 站点用的是私有 CA 自签证书，并且会定期轮换。证书一旦变化，旧的 HSTS
//! 策略（chrome://net-internals 里看到的 domain security policy）会强制走 HTTPS 并禁止
//! 用户「忽略证书错误」继续访问 —— 这就是每次换新证书都得手动去删 HSTS 的根因。
//!
//! 这里从 `PlatformWebview` 拿 controller 手动挂 `ServerCertificateErrorDetected`，对站点
//! 窗口里的证书错误一律 `ALWAYS_ALLOW`，等价于 Edge 里对该站点点「继续（不安全）」。
//! 这样证书轮换后无需再手动清 HSTS，站点窗口也能照常打开。
//!
//! 作用范围：仅 `create_site_window` 里的远程 SAS 站点窗口（本地设置窗口是 http，不会触发）。
//! 仅针对用户自己配置的 SAS 站点地址生效，属于内部工具可接受的安全取舍。
//!
//! ## 清除站点 HSTS 缓存（`clear_hsts` / `clear_hsts_pending`）
//!
//! WebView2 **没有**清除 HSTS 的公开 API：`COREWEBVIEW2_BROWSING_DATA_KINDS_*` 里没有
//! 对应项（那是 cookie / 缓存 / localStorage 那一套），只能动它磁盘上的存储：
//!
//!   <UDF>/EBWebView/<Profile>/Network/TransportSecurity
//!   （Windows 上 UDF = <应用本地数据目录>，实测 %LOCALAPPDATA%\com.saspwa）
//!
//! 该文件是 JSON，例如
//!   {"sts":[{"expiry":…,"host":"7/RG0z…=","mode":"force-https",…}],"version":2}
//! 注意 `host` 是 **base64(SHA-256)** 而不是明文域名（新版 Chromium 不落明文主机名），
//! 想「只删某个站点」就得现算哈希，还随 Chromium 版本可能变动 —— 不划算。而这份 UDF 是
//! 本客户端独占的，里面只可能存用户自己配置的 SAS 站点，所以直接删整个文件即可，
//! 效果等价于 Chrome net-internals 里对所有站点执行 Delete。
//!
//! ### 为什么要点菜单后留标记、下次启动才删
//! 本客户端是常驻进程，站点窗口「关闭」只是收进托盘，WebView 一直活着；本地设置窗口也用
//! 同一个 profile。因此：
//!   1. profile 一旦加载，`TransportSecurity` 就被占用，当场删多半 `Access is denied`；
//!   2. 就算删成功，**内存里的旧 HSTS 条目并不受影响**，且退出时会写回磁盘 —— 等于白删。
//! 可靠时机只有「进程启动、但还没有任何 WebView 创建」这一刻（`setup` 最开头，早于设置
//! 窗口与默认站点窗口）。于是菜单项的做法是：当场尽力删一次 + 无论如何留一个标记文件，
//! 下次开工由 `clear_hsts_pending` 在任何窗口创建之前完成真正的清理并清掉标记。

use tauri::AppHandle;

/// HSTS 存储文件名（Chromium TransportSecurityState 的磁盘格式，各 profile 一份）。
const HSTS_FILE: &str = "TransportSecurity";
/// 「下次启动清 HSTS」的标记文件，放在应用配置目录里（与 config.json 同级）。
const HSTS_PENDING: &str = "clear-hsts.pending";

/// 给站点窗口装上证书错误处理：证书异常一律放行。失败只记日志，不影响窗口使用。
#[cfg(windows)]
pub fn install(window: &tauri::WebviewWindow, app: AppHandle) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW, ICoreWebView2_14,
    };
    use webview2_com::ServerCertificateErrorDetectedEventHandler;
    use windows_core::Interface;

    let label = window.label().to_string();
    window
        .with_webview(move |pw| {
            let controller = pw.controller();
            // 回调跑在 WebView2 的 UI 线程上：只做「放行 + 记一行日志」，不做重活。
            let handler = ServerCertificateErrorDetectedEventHandler::create(Box::new(
                move |_sender, args| {
                    let Some(args) = args else { return Ok(()) };
                    // webview2-com 0.38 把 EventArgs 上的方法都标成了 unsafe（COM 约定）。
                    unsafe {
                        args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW)?;
                    }
                    crate::log_line(&app, &format!("cert error ignored (ALLOW): {label}"));
                    Ok(())
                },
            ));
            unsafe {
                let Ok(core) = controller.CoreWebView2() else {
                    return;
                };
                // add_ServerCertificateErrorDetected 定义在版本化接口 `ICoreWebView2_14` 上，
                // 不在 `ICoreWebView2` 本身上，要先 cast（wry 里也是这么取 13/19/22 的）。
                let Ok(core14) = core.cast::<ICoreWebView2_14>() else {
                    return;
                };
                let mut token = 0i64;
                let _ = core14.add_ServerCertificateErrorDetected(&handler, &mut token);
            }
        })
        .map_err(|e| format!("安装证书错误处理失败：{e}"))
}

/// WebView2 用户数据目录（UDF）：<应用本地数据目录>/EBWebView。
///
/// 实测落在 `%LOCALAPPDATA%\com.saspwa\EBWebView`。不同 Tauri 版本 / 打包配置下这个根目录
/// 可能落在本地数据目录或漫游数据目录，这里两个候选都试一遍，取第一个真实存在的。
#[cfg(windows)]
fn udf_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;

    let local = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("无法定位应用本地数据目录：{e}"))?;
    if let Ok(roaming) = app.path().data_dir() {
        let candidate = roaming.join("EBWebView");
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    // UDF 还没建（首次运行、还没开过站点窗口）：这时也没什么 HSTS 可清。
    Ok(local.join("EBWebView"))
}

/// 删除 UDF 下**所有** profile 的 HSTS 存储，返回实际删掉的文件数。
///
/// 为什么不逐个站点删：见模块顶部说明，`host` 是哈希值不是明文。
#[cfg(windows)]
pub fn clear_hsts(app: &AppHandle) -> Result<usize, String> {
    use std::fs;

    let udf = udf_dir(app)?;
    if !udf.is_dir() {
        return Ok(0);
    }
    let mut removed = 0usize;
    let entries = fs::read_dir(&udf).map_err(|e| format!("读取 {udf:?} 失败：{e}"))?;
    for entry in entries.flatten() {
        let profile = entry.path();
        if !profile.is_dir() {
            continue;
        }
        let target = profile.join("Network").join(HSTS_FILE);
        if target.is_file() {
            match fs::remove_file(&target) {
                Ok(()) => removed += 1,
                // 绝大多数情况就是文件被占用（Portable profile 还活着），交给下次启动处理。
                Err(e) => return Err(format!("删除 {target:?} 失败：{e}")),
            }
        }
    }
    Ok(removed)
}

/// 记下「下次启动请清 HSTS」。
#[cfg(windows)]
pub fn mark_clear_hsts_pending(app: &AppHandle) -> Result<(), String> {
    use std::fs;
    let path = crate::config::app_dir(app)?.join(HSTS_PENDING);
    fs::write(&path, "1").map_err(|e| format!("写入 {path:?} 失败：{e}"))
}

/// 启动时调用：**必须早于任何窗口创建**（含设置窗口）。
///
/// 有标记就删 HSTS 并清掉标记；返回实际删掉的文件数（无标记返回 None）。
#[cfg(windows)]
pub fn clear_hsts_pending(app: &AppHandle) -> Result<Option<usize>, String> {
    use std::fs;
    let path = crate::config::app_dir(app)?.join(HSTS_PENDING);
    if !path.is_file() {
        return Ok(None);
    }
    // 标记先抹掉：万一这次删除失败（文件异常被占），也不要下次再卡一轮。
    let _ = fs::remove_file(&path);
    Ok(Some(clear_hsts(app)?))
}

/// 非 Windows：Linux（WebKitGTK）由系统解析证书、也没有 EBWebView 目录可清，保持现状。
#[cfg(not(windows))]
pub fn install(_window: &tauri::WebviewWindow, _app: AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn clear_hsts(_app: &AppHandle) -> Result<usize, String> {
    Ok(0)
}

#[cfg(not(windows))]
pub fn mark_clear_hsts_pending(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn clear_hsts_pending(_app: &AppHandle) -> Result<Option<usize>, String> {
    Ok(None)
}
