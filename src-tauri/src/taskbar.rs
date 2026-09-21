//! Windows 任务栏身份（AppUserModelID）：让**每个站点窗口在任务栏上是独立的一项**
//! （独立分组、独立图标、可各自固定），而不是所有窗口挤在同一个 exe 图标底下 ——
//! 对齐 Edge PWA「每个应用一个任务栏项」的观感。非 Windows 平台是空实现。
//!
//! 为什么手写 FFI：`SHGetPropertyStoreForWindow` + `IPropertyStore` 只有 COM 接口，
//! 本项目一直不引入 `windows` / `winapi` 依赖（DPAPI 也是手写的，见 credentials.rs），
//! 这里沿用同样做法：按 ABI 声明最小可用的 vtable 结构。
//!
//! **关键时序**：`PKEY_AppUserModel_ID` 必须在窗口**第一次显示之前**写进去 —— Windows 是在
//! 窗口变可见时才创建任务栏按钮、并在此刻定格它的分组与图标。我们本来就是「隐藏创建 → 页面
//! 加载完成再 show」，所以调用点放在 `create_site_window` 里 `build()` 之后即可。
//!
//! 失败不影响窗口本身：调用方只记日志。

#![allow(non_snake_case)]
#![allow(dead_code)] // 非 Windows 平台这些声明用不到

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::path::Path;

    /// AppUserModel 属性的 FMTID：{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}
    const FMTID_APP_USER_MODEL: Guid = Guid {
        d1: 0x9F4C_2855,
        d2: 0x9F79,
        d3: 0x4B39,
        d4: [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3],
    };

    /// PKEY_AppUserModel_* 的 pid（见 propkey.h）
    const PID_RELAUNCH_COMMAND: u32 = 2;
    const PID_RELAUNCH_DISPLAY_NAME: u32 = 4;
    const PID_ID: u32 = 5;

    /// IID_IPropertyStore {886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99}
    const IID_PROPERTY_STORE: Guid = Guid {
        d1: 0x886D_8EEB,
        d2: 0x8CF2,
        d3: 0x4446,
        d4: [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99],
    };

    const VT_LPWSTR: u16 = 31;
    /// MTA：不需要消息泵，适合后台线程。
    const COINIT_MULTITHREADED: u32 = 0x0;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        d1: u32,
        d2: u16,
        d3: u16,
        d4: [u8; 8],
    }

    #[repr(C)]
    struct PropertyKey {
        fmtid: Guid,
        pid: u32,
    }

    /// PROPVARIANT：x64 下 24 字节 = 8 字节头（vt + 3×WORD 保留）+ 16 字节 union
    /// （union 因为含 DECIMAL 所以是 16 字节）。我们只用 VT_LPWSTR，只声明指针那段再补齐。
    #[repr(C)]
    struct PropVariant {
        vt: u16,
        _reserved: [u16; 3],
        pwsz: *mut u16,
        _rest: [u64; 1],
    }

    #[repr(C)]
    struct IUnknownVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct IPropertyStoreVtbl {
        base: IUnknownVtbl,
        get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
        get_at: unsafe extern "system" fn(*mut c_void, u32, *mut PropertyKey) -> i32,
        get_value: unsafe extern "system" fn(*mut c_void, *const PropertyKey, *mut PropVariant) -> i32,
        set_value:
            unsafe extern "system" fn(*mut c_void, *const PropertyKey, *const PropVariant) -> i32,
        commit: unsafe extern "system" fn(*mut c_void) -> i32,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetPropertyStoreForWindow(
            hwnd: *mut c_void,
            riid: *const Guid,
            property_store: *mut *mut c_void,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pv: *mut c_void, coinit: u32) -> i32;
        fn CoUninitialize();
    }

    fn set_string(
        vtbl: *mut IPropertyStoreVtbl,
        store: *mut c_void,
        pid: u32,
        value: &str,
    ) -> Result<(), String> {
        let mut wide: Vec<u16> = value.encode_utf16().collect();
        wide.push(0);
        let key = PropertyKey {
            fmtid: FMTID_APP_USER_MODEL,
            pid,
        };
        let pv = PropVariant {
            vt: VT_LPWSTR,
            _reserved: [0; 3],
            pwsz: wide.as_mut_ptr(),
            _rest: [0],
        };
        let hr = unsafe { ((*vtbl).set_value)(store, &key, &pv) };
        if hr < 0 {
            Err(format!("SetValue(pid={pid}) 失败：0x{hr:08X}"))
        } else {
            Ok(())
        }
    }

    fn set_props(
        hwnd: *mut c_void,
        site_id: &str,
        site_name: &str,
        exe: &Path,
    ) -> Result<(), String> {
        // AUMID 规则：≤128 字符，且只能含字母数字与 `.` `-` `_`（站点 id 在 config 侧已 sanitize）。
        let aumid = format!("com.saspwa.site.{site_id}");
        // 固定到任务栏后重新打开用：指向本程序（不带参数 = 走「默认站点」逻辑）。
        let relaunch = format!("\"{}\"", exe.display());

        unsafe {
            let mut store: *mut c_void = std::ptr::null_mut();
            let hr = SHGetPropertyStoreForWindow(hwnd, &IID_PROPERTY_STORE, &mut store);
            if hr < 0 || store.is_null() {
                return Err(format!("SHGetPropertyStoreForWindow 失败：0x{hr:08X}"));
            }
            let vtbl = *(store as *mut *mut IPropertyStoreVtbl);

            let mut first_err = None;
            let mut remember = |r: Result<(), String>| {
                if first_err.is_none() {
                    if let Err(e) = r {
                        first_err = Some(e);
                    }
                }
            };
            remember(set_string(vtbl, store, PID_ID, &aumid));
            remember(set_string(vtbl, store, PID_RELAUNCH_COMMAND, &relaunch));
            remember(set_string(vtbl, store, PID_RELAUNCH_DISPLAY_NAME, site_name));
            // 一次 Commit 落盘属性（不 Commit 不生效）。
            let _ = ((*vtbl).commit)(store);
            let _ = ((*vtbl).base.release)(store);
            match first_err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
    }

    /// 给站点窗口设置独立的任务栏身份（分组 / 固定 / 图标回落）。
    pub fn apply_site_identity(
        hwnd: *mut c_void,
        site_id: &str,
        site_name: &str,
        exe: &Path,
    ) -> Result<(), String> {
        if hwnd.is_null() {
            return Err("窗口句柄为空".into());
        }
        // COM 需要在本线程初始化；S_OK(0) = 本次初始化成功（结束时反初始化），
        // S_FALSE(1) = 本线程早已初始化（此时不能反初始化）。
        let hr = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_MULTITHREADED) };
        let owned = hr == 0;
        let result = set_props(hwnd, site_id, site_name, exe);
        if owned {
            unsafe { CoUninitialize() };
        }
        result
    }
}

#[cfg(windows)]
pub use imp::apply_site_identity;

/// 非 Windows：任务栏/分组这套是 Windows 的概念，什么也不做。
#[cfg(not(windows))]
pub fn apply_site_identity(
    _hwnd: *mut std::ffi::c_void,
    _site_id: &str,
    _site_name: &str,
    _exe: &std::path::Path,
) -> Result<(), String> {
    Ok(())
}
