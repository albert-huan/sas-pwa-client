//! 站点窗口 F11 → 切换无边框的**原生兜底**实现（Windows 低级键盘钩子）。
//!
//! 为什么不能只靠注入脚本：WebView2 默认把 F11 当作「浏览器加速键」自行处理
//! （wry 的 `browser_accelerator_keys` 默认为 true，Tauri 没有开放这个开关），
//! 按键不一定送到页面 DOM，注入脚本里的 keydown 监听就收不到。
//! 钩子层抢下 F11 最可靠：只有「前台窗口是本站点窗口」时才拦截，其余一律放行。
//!
//! 注意：钩子回调运行在安装线程（主线程）的消息泵里，**必须立刻返回**，
//! 否则系统会静默摘掉这个钩子；因此真正的切换动作投递到后台线程执行。

use std::sync::OnceLock;

/// 触发切换的回调：参数是站点窗口的 label（`site-<id>`）。
pub type ToggleFn = fn(&tauri::AppHandle, &str);

struct State {
    app: tauri::AppHandle,
    toggle: ToggleFn,
}

static STATE: OnceLock<State> = OnceLock::new();

/// 安装 F11 钩子；返回是否成功（失败时由注入脚本里的 keydown 监听兜底）。
pub fn install(app: tauri::AppHandle, toggle: ToggleFn) -> bool {
    if STATE.set(State { app, toggle }).is_err() {
        return false; // 已经装过了
    }
    #[cfg(windows)]
    {
        unsafe { imp::install() }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn state() -> Option<&'static State> {
    STATE.get()
}

/// 当前前台窗口若是某个站点窗口，返回它的 label。
///
/// `hwnd()` 在主线程上是就地执行的（tauri-runtime-wry 的 `send_user_message`
/// 对主线程走内联分支），所以这里不会像异步 getter 那样卡住。
#[cfg(windows)]
fn foreground_site_label(app: &tauri::AppHandle) -> Option<String> {
    use tauri::Manager;

    let fg = unsafe { imp::GetForegroundWindow() };
    if fg == 0 {
        return None;
    }
    let root = unsafe { imp::GetAncestor(fg, imp::GA_ROOT) };
    let target = if root != 0 { root } else { fg };

    app.webview_windows().into_iter().find_map(|(label, w)| {
        if !label.starts_with(crate::WINDOW_PREFIX) {
            return None;
        }
        let hwnd = w.hwnd().map(|h| h.0 as isize).unwrap_or(0);
        (hwnd != 0 && hwnd == target).then_some(label)
    })
}

#[cfg(windows)]
mod imp {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::{foreground_site_label, state};

    pub const GA_ROOT: u32 = 2;

    /// 按住 F11（或某些注入方式的按键）时系统会持续回放 keydown，
    /// 只在「按下沿」触发一次：keydown 触发后置位，keyup 复位；
    /// 万一 keyup 丢了，超过 `KEY_REPEAT_GUARD` 也算一次新的按下。
    static HELD_AT: Mutex<Option<Instant>> = Mutex::new(None);
    const KEY_REPEAT_GUARD: Duration = Duration::from_millis(1200);

    fn take_press() -> bool {
        let mut held = HELD_AT.lock().unwrap();
        let now = Instant::now();
        if let Some(at) = *held {
            if now.duration_since(at) < KEY_REPEAT_GUARD {
                return false;
            }
        }
        *held = Some(now);
        true
    }

    fn release_press() {
        *HELD_AT.lock().unwrap() = None;
    }

    const WH_KEYBOARD_LL: i32 = 13;
    const HC_ACTION: i32 = 0;
    const WM_KEYDOWN: usize = 0x0100;
    const WM_KEYUP: usize = 0x0101;
    const WM_SYSKEYDOWN: usize = 0x0104;
    const WM_SYSKEYUP: usize = 0x0105;
    const VK_F11: u32 = 0x7A;

    /// https://learn.microsoft.com/windows/win32/api/winuser/ns-winuser-kbdllhookstruct
    #[repr(C)]
    struct KbdLlHookStruct {
        vk_code: u32,
        scan_code: u32,
        flags: u32,
        time: u32,
        dw_extra_info: usize,
    }

    type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowsHookExW(id_hook: i32, lpfn: HookProc, hmod: isize, thread_id: u32) -> isize;
        fn CallNextHookEx(hhk: isize, code: i32, wparam: usize, lparam: isize) -> isize;
        pub(super) fn GetForegroundWindow() -> isize;
        pub(super) fn GetAncestor(hwnd: isize, flags: u32) -> isize;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
    }

    pub(super) unsafe fn install() -> bool {
        let hmod = GetModuleHandleW(std::ptr::null());
        // thread_id = 0 → 全局钩子；低级键盘钩子的回调不需要 DLL，进程内即可。
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, hook_proc, hmod, 0);
        hook != 0
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        if code == HC_ACTION && lparam != 0 {
            let info = unsafe { &*(lparam as *const KbdLlHookStruct) };
            if info.vk_code == VK_F11 {
                if wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN {
                    // 只有前台是站点窗口时才接管；长按的重复事件照样吞掉，但不切换。
                    if let Some(s) = state() {
                        if let Some(label) = foreground_site_label(&s.app) {
                            if take_press() {
                                let app = s.app.clone();
                                let toggle = s.toggle;
                                std::thread::spawn(move || toggle(&app, &label));
                            }
                            return 1; // 吞掉按键，避免 WebView2 自己也切全屏
                        }
                    }
                } else if wparam == WM_KEYUP || wparam == WM_SYSKEYUP {
                    release_press();
                }
            }
        }
        unsafe { CallNextHookEx(0, code, wparam, lparam) }
    }
}
