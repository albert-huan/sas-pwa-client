//! 站点窗口的原生键盘兜底实现（Windows 低级键盘钩子）。
//!
//! 为什么不能只靠注入脚本：WebView2 默认把 F11 当作「浏览器加速键」自行处理
//! （wry 的 `browser_accelerator_keys` 默认为 true，Tauri 没有开放这个开关），
//! 按键不一定送到页面 DOM，注入脚本里的 keydown 监听就收不到。
//! 钩子层抢下按键最可靠：只有「前台窗口是本站点窗口」时才拦截，其余一律放行。
//!
//! 目前接管两个键（注入脚本里都有 keydown 兜底，供 Linux 与钩子安装失败时使用）：
//!   * F11     → 切换无边框；
//!   * Ctrl+`  → 唤出 / 收起顶部居中的悬浮窗口栏。
//!
//! 注意：钩子回调运行在安装线程（主线程）的消息泵里，**必须立刻返回**，
//! 否则系统会静默摘掉这个钩子；因此真正的动作投递到后台线程执行。

use std::sync::OnceLock;

/// 钩子识别到的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleFrameless,
    ToggleDock,
}

/// 触发动作的回调：参数是站点窗口的 label（`site-<id>`）与动作。
pub type TriggerFn = fn(&tauri::AppHandle, &str, Action);

struct State {
    app: tauri::AppHandle,
    trigger: TriggerFn,
}

static STATE: OnceLock<State> = OnceLock::new();

/// 安装键盘钩子；返回是否成功（失败时由注入脚本里的 keydown 监听兜底）。
pub fn install(app: tauri::AppHandle, trigger: TriggerFn) -> bool {
    if STATE.set(State { app, trigger }).is_err() {
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
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::{foreground_site_label, state, Action};

    pub const GA_ROOT: u32 = 2;

    /// 按住不放时系统会持续回放 keydown。按虚拟键码分别记下按下时刻，
    /// 只在「按下沿」触发一次；keyup 复位，万一 keyup 丢了超过 guard 也算新的一次。
    static HELD_AT: Mutex<Option<HashMap<u32, Instant>>> = Mutex::new(None);
    const KEY_REPEAT_GUARD: Duration = Duration::from_millis(1200);

    fn take_press(vk: u32) -> bool {
        let mut guard = HELD_AT.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        let now = Instant::now();
        if let Some(at) = map.insert(vk, now) {
            if now.duration_since(at) < KEY_REPEAT_GUARD {
                return false;
            }
        }
        true
    }

    fn release_press(vk: u32) {
        if let Ok(mut guard) = HELD_AT.lock() {
            if let Some(map) = guard.as_mut() {
                map.remove(&vk);
            }
        }
    }

    const WH_KEYBOARD_LL: i32 = 13;
    const HC_ACTION: i32 = 0;
    const WM_KEYDOWN: usize = 0x0100;
    const WM_KEYUP: usize = 0x0101;
    const WM_SYSKEYDOWN: usize = 0x0104;
    const WM_SYSKEYUP: usize = 0x0105;
    const VK_F11: u32 = 0x7A;
    const VK_OEM_3: u32 = 0xC0; // ` / ~ 键
    const VK_CONTROL: i32 = 0x11;

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
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
    }

    fn ctrl_down() -> bool {
        (unsafe { GetAsyncKeyState(VK_CONTROL) } as u16) & 0x8000 != 0
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
            let action = match info.vk_code {
                VK_F11 => Some(Action::ToggleFrameless),
                VK_OEM_3 if ctrl_down() => Some(Action::ToggleDock),
                _ => None,
            };
            if let Some(action) = action {
                let up = wparam == WM_KEYUP || wparam == WM_SYSKEYUP;
                let down = wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN;
                if up {
                    release_press(info.vk_code);
                }
                if down {
                    // 只有前台是站点窗口时才接管；长按的重复事件照样吞掉，但不重复触发。
                    if let Some(s) = state() {
                        if let Some(label) = foreground_site_label(&s.app) {
                            if take_press(info.vk_code) {
                                let app = s.app.clone();
                                let trigger = s.trigger;
                                std::thread::spawn(move || trigger(&app, &label, action));
                            }
                            return 1; // 吞掉按键，避免 WebView2 自己也处理
                        }
                    }
                }
            }
        }
        unsafe { CallNextHookEx(0, code, wparam, lparam) }
    }
}
