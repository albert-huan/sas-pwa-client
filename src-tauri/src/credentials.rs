// 站点登录凭据（用户名 / 密码）的本地保存。
//
// 密码不以明文落盘：Windows 下走 DPAPI（CryptProtectData / CryptUnprotectData），
// 密钥由系统按「当前 Windows 用户 + 当前机器」托管，因此：
//   - 同一台电脑上的其它用户账户解不开；
//   - 把 credentials.json 拷到别的机器也解不开。
// 文件与 config.json 同目录：%APPDATA%\sas-pwa-client\credentials.json，密文按十六进制存放。
// 另外还带一段固定附加熵（ENTROPY），别的程序即便同用户也解不出我们的密文。
//
// 边界说明：为了在登录页自动填写，解密后的明文只存在于本进程内存，以及对应站点窗口的注入脚本里，
// 与浏览器「记住密码」的暴露面相当；这里只保证「磁盘上不是明文」。
// 不引入任何第三方 crate：DPAPI 走自己的 FFI 声明（crypt32 / kernel32 系统库）。

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::config;

const CRED_FILE: &str = "credentials.json";

/// 单条凭据（一个站点一份）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Credential {
    pub site_id: String,
    pub username: String,
    /// DPAPI 密文的十六进制；空串表示只记了用户名、没存密码。
    pub secret: String,
    /// 是否在登录页自动提交表单（自动登录）。
    pub auto_login: bool,
}

/// 返回给设置页的视图：只回用户名与「是否存了密码」，绝不下发密文或明文密码。
#[derive(Debug, Clone, Default, Serialize)]
pub struct CredentialView {
    pub username: String,
    pub has_password: bool,
    pub auto_login: bool,
}

impl Credential {
    fn view(&self) -> CredentialView {
        CredentialView {
            username: self.username.clone(),
            has_password: !self.secret.is_empty(),
            auto_login: self.auto_login,
        }
    }
}

fn cred_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config::app_dir(app)?.join(CRED_FILE))
}

/// 读取全部凭据；文件不存在或损坏时返回空表（不阻断启动）。
pub fn load(app: &AppHandle) -> Vec<Credential> {
    let path = match cred_path(app) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<Credential>>(&text) {
        Ok(list) => list,
        Err(e) => {
            eprintln!("[SAS PWA 客户端] 凭据文件解析失败，按空处理：{e}");
            Vec::new()
        }
    }
}

fn save_all(app: &AppHandle, list: &[Credential]) -> Result<(), String> {
    let path = cred_path(app)?;
    let text = serde_json::to_string_pretty(list).map_err(|e| format!("序列化凭据失败：{e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入凭据文件 {path:?} 失败：{e}"))
}

/// 查某站点的凭据（含密文，仅供内部使用）。
pub fn find(app: &AppHandle, site_id: &str) -> Option<Credential> {
    load(app).into_iter().find(|c| c.site_id == site_id)
}

/// 查某站点的凭据视图（给设置页用）。
pub fn view(app: &AppHandle, site_id: &str) -> Option<CredentialView> {
    find(app, site_id).map(|c| c.view())
}

/// 保存 / 更新凭据。
///
/// - `username` 为空 → 视为清除该站点凭据；
/// - `password` 为 `None` 或空串 → 保留已保存的密码（只改用户名 / 自动登录开关）。
pub fn upsert(
    app: &AppHandle,
    site_id: &str,
    username: &str,
    password: Option<&str>,
    auto_login: bool,
) -> Result<CredentialView, String> {
    let username = username.trim();
    if username.is_empty() {
        remove(app, site_id)?;
        return Ok(CredentialView::default());
    }

    let mut list = load(app);
    let secret = match password {
        Some(pw) if !pw.is_empty() => {
            let cipher = dpapi::protect(pw)?;
            to_hex(&cipher)
        }
        _ => list
            .iter()
            .find(|c| c.site_id == site_id)
            .map(|c| c.secret.clone())
            .unwrap_or_default(),
    };

    let entry = Credential {
        site_id: site_id.to_string(),
        username: username.to_string(),
        secret,
        auto_login,
    };
    match list.iter_mut().find(|c| c.site_id == site_id) {
        Some(old) => *old = entry.clone(),
        None => list.push(entry.clone()),
    }
    save_all(app, &list)?;
    Ok(entry.view())
}

/// 删除某站点的凭据。
pub fn remove(app: &AppHandle, site_id: &str) -> Result<(), String> {
    let mut list = load(app);
    let before = list.len();
    list.retain(|c| c.site_id != site_id);
    if list.len() != before {
        save_all(app, &list)?;
    }
    Ok(())
}

/// 清理配置里已不存在的站点凭据（删除站点后不留孤儿记录）。
pub fn prune(app: &AppHandle, alive: &[String]) -> Result<(), String> {
    let list = load(app);
    let before = list.len();
    let kept: Vec<Credential> = list
        .into_iter()
        .filter(|c| alive.iter().any(|a| a == &c.site_id))
        .collect();
    if kept.len() != before {
        save_all(app, &kept)?;
    }
    Ok(())
}

/// 解密出可用于自动填写登录页的三元组：(用户名, 密码, 是否自动提交)。
/// 没保存密码、或解密失败（换机器 / 换用户 / 文件被改）时返回 None。
pub fn login_of(app: &AppHandle, site_id: &str) -> Option<(String, String, bool)> {
    let cred = find(app, site_id)?;
    if cred.secret.is_empty() {
        return None;
    }
    let cipher = match from_hex(&cred.secret) {
        Some(c) => c,
        None => {
            eprintln!("[SAS PWA 客户端] 凭据密文格式异常（{site_id}）");
            return None;
        }
    };
    match dpapi::unprotect(&cipher) {
        Ok(password) => Some((cred.username, password, cred.auto_login)),
        Err(e) => {
            eprintln!("[SAS PWA 客户端] 凭据解密失败（{site_id}）：{e}");
            None
        }
    }
}

// ---------------- 十六进制编解码（避免为几个字节引入 base64 依赖） ----------------

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

// ---------------- 加密：Windows DPAPI（其余平台退化为明文，仅保证能编译） ----------------

#[cfg(windows)]
mod dpapi {
    use std::ffi::c_void;

    /// 对应 Win32 的 DATA_BLOB / CRYPT_INTEGER_BLOB。
    #[repr(C)]
    struct Blob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *const Blob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const Blob,
            pv_reserved: *const c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut Blob,
        ) -> i32;

        fn CryptUnprotectData(
            p_data_in: *const Blob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const Blob,
            pv_reserved: *const c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut Blob,
        ) -> i32;
    }

    // LocalFree 用来释放 DPAPI 分配的输出缓冲。
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
    }

    /// 固定附加熵：让密文只能被本程序解开（同用户下的其它程序也解不出）。
    const ENTROPY: &[u8] = b"sas-pwa-client/credentials/v1";

    fn blob(data: &[u8]) -> Blob {
        Blob {
            cb_data: data.len() as u32,
            pb_data: data.as_ptr() as *mut u8,
        }
    }

    /// 取走输出缓冲的内容并释放它。
    fn take(out: Blob) -> Vec<u8> {
        if out.pb_data.is_null() || out.cb_data == 0 {
            return Vec::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(out.pb_data, out.cb_data as usize).to_vec() };
        unsafe {
            LocalFree(out.pb_data as *mut c_void);
        }
        bytes
    }

    fn empty_blob() -> Blob {
        Blob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        }
    }

    pub fn protect(plain: &str) -> Result<Vec<u8>, String> {
        let entropy = blob(ENTROPY);
        let data = blob(plain.as_bytes());
        let mut out = empty_blob();
        let ok = unsafe {
            CryptProtectData(
                &data,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(format!(
                "DPAPI 加密失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(take(out))
    }

    pub fn unprotect(cipher: &[u8]) -> Result<String, String> {
        let entropy = blob(ENTROPY);
        let data = blob(cipher);
        let mut out = empty_blob();
        let ok = unsafe {
            CryptUnprotectData(
                &data,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(format!(
                "DPAPI 解密失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        String::from_utf8(take(out)).map_err(|e| format!("解密结果不是合法文本：{e}"))
    }
}

#[cfg(not(windows))]
mod dpapi {
    // 非 Windows 平台没有 DPAPI；本项目只发布 Windows 版，这里退化为明文存储以保证能编译。
    pub fn protect(plain: &str) -> Result<Vec<u8>, String> {
        Ok(plain.as_bytes().to_vec())
    }

    pub fn unprotect(cipher: &[u8]) -> Result<String, String> {
        String::from_utf8(cipher.to_vec()).map_err(|e| format!("解密结果不是合法文本：{e}"))
    }
}
