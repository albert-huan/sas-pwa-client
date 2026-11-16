// 配置持久化：把用户填写的 SAS 站点清单保存到应用配置目录下的 config.json。
//
// 配置目录 = <用户配置目录>/<包名>/，即 Windows 下为 %APPDATA%\sas-pwa-client\，
// 与程序（exe）名称保持一致，方便按程序名直接找到配置；
// 不沿用 Tauri 默认的 %APPDATA%/<identifier>/。不需要任何文件系统插件。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const CONFIG_FILE: &str = "config.json";

/// 配置目录名：Cargo 包名，即程序/exe 名（sas-pwa-client）。
const APP_FOLDER_NAME: &str = env!("CARGO_PKG_NAME");

// 说明：早期配置目录是按 bundle identifier 命名的，改过几次名字。
// 这里不再写死任何历史目录名，改为扫描用户配置目录下的其它目录，
// 用内容判断是不是本应用的 config.json（见 `looks_like_our_config`），
// 命中就搬过来，避免老用户丢配置。

/// 单个 SAS 站点的**连接配置**（存盘数据，不是运行实例）。
///
/// 一个站点可以同时开多个窗口：窗口标签是 `site-<id>`、`site-<id>-2`…，
/// 由 `WINDOW_SITES` 注册表维护归属；本结构体里没有任何窗口状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Site {
    /// 站点标识：窗口标签的前缀、tray 菜单 id、凭据键与任务栏 AUMID 都用它。
    pub id: String,
    /// 展示名称，用作窗口标题与 tray 菜单文案。
    pub name: String,
    /// 访问地址（http/https）。
    pub url: String,
    /// 是否伪装成 PWA 并周期性派发活动脉冲，避免 SAS 前端空闲超时踢下线。
    pub keep_awake: bool,
    /// 活动脉冲间隔（秒），0 表示不派发脉冲。
    pub pulse_seconds: u64,
    /// 无边框窗口（隐藏系统标题栏，改用注入的自绘标题条）。
    pub frameless: bool,
    /// 默认登录环境：设置后下次启动自动连接该站点（整份配置里最多一个）。
    pub default: bool,
}

impl Default for Site {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            keep_awake: true,
            pulse_seconds: 120,
            frameless: false,
            default: false,
        }
    }
}

/// 应用整体配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub sites: Vec<Site>,
    /// 最近一次打开的站点 id，启动时自动恢复。
    pub last_site_id: Option<String>,
    /// UI 主题偏好：system（跟随系统）/ light / dark。
    pub ui_theme: String,
    /// Linux 渲染模式：auto（自动，见 lib.rs::tune_linux_webkit）/ smooth（流畅优先，保持
    /// DMA-BUF 硬件加速）/ compat（兼容优先，关闭 DMA-BUF 走共享内存）。
    /// 实质是 WebKitGTK 的环境变量，必须在 WebKitGTK 初始化之前定下来，所以改完要重启客户端。
    pub render_mode: String,
    /// 是否允许在窗口里打开开发者工具（F12 / 右键检查）。**默认关闭**：
    /// 站点窗口承载的是远程 SAS 页面，DevTools 一旦可用，能碰到这个窗口的人就能读到
    /// 注入脚本里用于自动填充的登录凭据（见 lib.rs::render_script）。只影响此后新建的窗口。
    pub dev_tools: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sites: Vec::new(),
            last_site_id: None,
            ui_theme: "system".to_string(),
            render_mode: DEFAULT_RENDER_MODE.to_string(),
            dev_tools: false,
        }
    }
}

/// 悬浮窗口栏的外观与位置偏好，单独存 dock.json（不放进 config.json：它不属于站点配置，
/// 且远程站点页面有权写它，隔离开可以保证页面最多只能改「样子」，碰不到站点地址）。
///
/// 位置用「可移动范围内的比例」表示：`cx`/`cy` ∈ [0,1]，0 = 贴左 / 贴上，1 = 贴右 / 贴下。
/// 比例而不是像素，是因为各窗口大小不同，同一份偏好要能在所有窗口里落在相对一致的位置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockPrefs {
    pub cx: f64,
    pub cy: f64,
    /// 贴靠的边：""（悬浮在中间）/ top / bottom / left / right。
    pub edge: String,
    /// 主题 id：内置 midnight / graphite / forest / dusk / paper，或 custom（用下面两个颜色）。
    pub theme: String,
    /// 自定义主题的底色与强调色（#rrggbb）。内置主题由前端按 theme 查表，这两个值只在 custom 时生效。
    pub bg: String,
    pub accent: String,
    /// 背景不透明度 0.35 ~ 1.0（只影响底色，文字与图标始终不透明）。
    pub opacity: f64,
    /// 贴边时收起后完全隐藏，鼠标靠近原位置再显示。
    pub edge_hide: bool,
    /// 窗口别名映射：窗口标签（site-<id>[-n]）→ 用户起的别名。别名只影响显示
    /// （悬浮栏 chip / 任务栏标题），不参与归属判断。BTreeMap 保证 dock.json 里的
    /// 键序稳定，diff 友好。
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

impl Default for DockPrefs {
    fn default() -> Self {
        Self {
            cx: 0.5,
            cy: 0.0,
            // 默认不吸边：收起后是一枚始终可见的小把手。吸边（并在收起时完全隐藏）只发生在
            // 用户主动把胶囊拖到屏幕边缘之后；默认就隐藏的话，新装的用户会以为胶囊消失了。
            edge: String::new(),
            theme: "midnight".to_string(),
            bg: "#1b2233".to_string(),
            accent: "#7aa8ff".to_string(),
            opacity: 0.94,
            edge_hide: true,
            aliases: BTreeMap::new(),
        }
    }
}

const DOCK_THEMES: [&str; 6] = ["midnight", "graphite", "forest", "dusk", "paper", "custom"];
const DOCK_EDGES: [&str; 5] = ["", "top", "bottom", "left", "right"];

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl DockPrefs {
    /// 清洗不可信输入：这份偏好既来自磁盘（可能被手改），也来自远程页面里的 IPC 调用。
    /// 非法值一律回落到默认值，数值夹到合法范围；颜色只放行 #rrggbb，避免被拿去拼样式。
    pub fn sanitize(mut self) -> Self {
        let d = DockPrefs::default();
        let unit = |v: f64, fallback: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { fallback };
        self.cx = unit(self.cx, d.cx);
        self.cy = unit(self.cy, d.cy);
        if !DOCK_EDGES.contains(&self.edge.as_str()) {
            self.edge = d.edge.clone();
        }
        if !DOCK_THEMES.contains(&self.theme.as_str()) {
            self.theme = d.theme.clone();
        }
        if !is_hex_color(&self.bg) {
            self.bg = d.bg.clone();
        }
        if !is_hex_color(&self.accent) {
            self.accent = d.accent.clone();
        }
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.35, 1.0)
        } else {
            d.opacity
        };
        // 别名：只保留站点窗口标签，值 trim 后截到 24 个字符；空别名即删除该条。
        if self.aliases.len() > 200 {
            self.aliases.clear();
        }
        self.aliases.retain(|k, v| {
            if !k.starts_with("site-") {
                return false;
            }
            let t = v.trim().chars().take(24).collect::<String>();
            if t.is_empty() {
                return false;
            }
            *v = t;
            true
        });
        self
    }
}

const DOCK_PREFS_FILE: &str = "dock.json";

/// 读取悬浮栏偏好；文件不存在或损坏时用默认值（外观偏好不值得阻断启动）。
pub fn load_dock_prefs(app: &AppHandle) -> DockPrefs {
    let Ok(dir) = app_dir(app) else {
        return DockPrefs::default();
    };
    fs::read_to_string(dir.join(DOCK_PREFS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<DockPrefs>(&t).ok())
        .unwrap_or_default()
        .sanitize()
}

pub fn save_dock_prefs(app: &AppHandle, prefs: &DockPrefs) -> Result<(), String> {
    let path = app_dir(app)?.join(DOCK_PREFS_FILE);
    let text = serde_json::to_string_pretty(prefs).map_err(|e| format!("序列化悬浮栏偏好失败：{e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入 {path:?} 失败：{e}"))
}

/// 渲染模式的合法取值与默认值。
pub const RENDER_MODES: [&str; 3] = ["auto", "smooth", "compat"];
pub const DEFAULT_RENDER_MODE: &str = "auto";

/// 归一化渲染模式：未知值（含手工编辑配置写错）一律回落到 auto。
pub fn normalize_render_mode(mode: &str) -> String {
    let m = mode.trim().to_ascii_lowercase();
    if RENDER_MODES.contains(&m.as_str()) {
        m
    } else {
        DEFAULT_RENDER_MODE.to_string()
    }
}

/// 启动最早期读取渲染模式（Tauri 初始化之前，此时还没有 AppHandle）。
///
/// 为什么不用 `load()`：`WEBKIT_*` 环境变量必须在 WebKitGTK 初始化之前设好，而 `load()`
/// 依赖 AppHandle 才能定位配置目录；这里按 Linux 的约定自己拼一次路径（与 Tauri 的
/// `config_dir()` 一致：`$XDG_CONFIG_HOME` 或 `$HOME/.config`）。读不到就按默认值来。
#[cfg(target_os = "linux")]
pub fn pre_init_render_mode() -> String {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let Some(dir) = dir else {
        return DEFAULT_RENDER_MODE.to_string();
    };
    let Ok(text) = fs::read_to_string(dir.join(APP_FOLDER_NAME).join(CONFIG_FILE)) else {
        return DEFAULT_RENDER_MODE.to_string();
    };
    let mode = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("render_mode").and_then(|m| m.as_str().map(str::to_string)))
        .unwrap_or_default();
    normalize_render_mode(&mode)
}

/// 用户级配置根目录（Windows：%APPDATA%，Linux：~/.config）。
fn user_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .config_dir()
        .map_err(|e| format!("无法定位用户配置目录：{e}"))
}

/// 应用配置目录（<用户配置目录>/<程序名>/，必要时创建）：config.json 与 credentials.json 都放这里。
pub fn app_dir(app: &AppHandle) -> Result<PathBuf, String> {
    // 目录名取 Cargo 包名（= 程序/exe 名 sas-pwa-client）。
    // 注意别用 `package_info().name`：那是展示用的 productName（这里是 "SAS"）。
    let dir = user_config_dir(app)?.join(APP_FOLDER_NAME);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录 {dir:?}：{e}"))?;
    Ok(dir)
}

/// 配置文件路径（必要时创建目录）：<用户配置目录>/<程序名>/config.json
pub fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_dir(app)?.join(CONFIG_FILE))
}

/// 内容看起来像本应用的配置（含 sites 数组且至少一项有 url）。
fn looks_like_our_config(text: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => v
            .get("sites")
            .and_then(|s| s.as_array())
            .map(|list| {
                list.iter().any(|item| {
                    item.get("url")
                        .and_then(|u| u.as_str())
                        .map(|u| !u.trim().is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// 若当前配置目录还没有配置文件，尝试从用户配置目录下的其它历史目录搬一次（只搬 config.json）。
fn migrate_legacy(base: &Path, path: &Path) {
    if path.exists() {
        return;
    }
    let current_dir = path.parent().map(|p| p.to_path_buf());
    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if current_dir.as_deref() == Some(dir.as_path()) {
            continue;
        }
        let old = dir.join(CONFIG_FILE);
        if !old.exists() {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&old) {
            if looks_like_our_config(&text) && fs::write(path, text).is_ok() {
                eprintln!("[SAS PWA 客户端] 已从旧配置目录迁移配置：{old:?}");
                break;
            }
        }
    }
}

/// 读取配置；文件不存在或格式错误时返回默认配置（不会中断启动）。
pub fn load(app: &AppHandle) -> AppConfig {
    let path = config_path(app);
    let base = user_config_dir(app);
    let mut cfg = match &path {
        Ok(p) => {
            if let Ok(b) = &base {
                migrate_legacy(b, p);
            }
            match fs::read_to_string(p) {
                Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[SAS PWA 客户端] 配置文件解析失败，将使用空配置：{e}");
                        AppConfig::default()
                    }
                },
                Err(_) => AppConfig::default(),
            }
        }
        Err(_) => AppConfig::default(),
    };
    // 兼容手工编辑过的文件：归一化失败只提示，不阻断。
    if let Err(e) = normalize(&mut cfg) {
        eprintln!("[SAS PWA 客户端] 配置归一化失败：{e}");
    }
    cfg
}

/// 写入配置。
pub fn save(app: &AppHandle, cfg: &AppConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化配置失败：{e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入配置文件 {path:?} 失败：{e}"))?;
    Ok(())
}

/// 校验并补全配置：补齐 URL 协议、生成/去重 id、规范名称、限制脉冲间隔、保证默认站点唯一。
pub fn normalize(cfg: &mut AppConfig) -> Result<(), String> {
    cfg.render_mode = normalize_render_mode(&cfg.render_mode);
    let mut used: Vec<String> = Vec::with_capacity(cfg.sites.len());

    for (i, site) in cfg.sites.iter_mut().enumerate() {
        let raw_url = site.url.trim().to_string();
        if !raw_url.is_empty() {
            let url = if raw_url.contains("://") {
                raw_url
            } else {
                format!("https://{raw_url}")
            };
            let parsed = url::Url::parse(&url).map_err(|e| format!("第 {} 项地址无效（{url}）：{e}", i + 1))?;
            let scheme = parsed.scheme().to_ascii_lowercase();
            if scheme != "http" && scheme != "https" {
                return Err(format!("第 {} 项仅支持 http/https 地址：{url}", i + 1));
            }
            if parsed.host_str().map_or(true, |h| h.trim().is_empty()) {
                return Err(format!("第 {} 项地址缺少主机名：{url}", i + 1));
            }
            site.url = parsed.to_string();
            if site.name.trim().is_empty() {
                site.name = parsed.host_str().unwrap_or("站点").to_string();
            }
        } else if site.name.trim().is_empty() {
            site.name = format!("站点 {}", i + 1);
        }

        site.name = site.name.trim().to_string();
        site.pulse_seconds = site.pulse_seconds.min(3600);

        let base = sanitize_id(&site.id);
        let base = if base.is_empty() {
            let host = url::Url::parse(&site.url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()))
                .unwrap_or_else(|| format!("site{}", i + 1));
            sanitize_id(&host)
        } else {
            base
        };
        site.id = unique_id(&base, &mut used);
    }

    // 默认登录环境只允许一个：保留第一个，其余清掉（防止手工编辑出多个默认值）。
    let mut has_default = false;
    for site in cfg.sites.iter_mut() {
        if site.default {
            if has_default {
                site.default = false;
            } else {
                has_default = true;
            }
        }
    }

    Ok(())
}

/// 保留字母数字、`-`、`_`，其余字符替换为 `-`（id 会作为窗口标签）。
fn sanitize_id(input: &str) -> String {
    let s: String = input
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.trim_matches('-').to_ascii_lowercase()
}

/// 保证 id 在本次配置中唯一。
fn unique_id(base: &str, used: &mut Vec<String>) -> String {
    let base = if base.is_empty() { "site" } else { base };
    let mut candidate = base.to_string();
    let mut n = 1usize;
    while used.iter().any(|u| u == &candidate) {
        n += 1;
        candidate = format!("{base}-{n}");
    }
    used.push(candidate.clone());
    candidate
}

#[cfg(test)]
mod dock_prefs_tests {
    use super::DockPrefs;
    use std::collections::BTreeMap;

    #[test]
    fn sanitize_clamps_and_rejects_bad_values() {
        let p = DockPrefs {
            cx: 9.0,
            cy: f64::NAN,
            edge: "diagonal".into(),
            theme: "<script>".into(),
            bg: "red; background:url(x)".into(),
            accent: "#12345".into(),
            opacity: 0.0,
            edge_hide: true,
            aliases: BTreeMap::new(),
        }
        .sanitize();
        let d = DockPrefs::default();
        assert_eq!(p.cx, 1.0);
        assert_eq!(p.cy, d.cy);
        assert_eq!(p.edge, d.edge);
        assert_eq!(p.theme, d.theme);
        assert_eq!(p.bg, d.bg);
        assert_eq!(p.accent, d.accent);
        assert_eq!(p.opacity, 0.35);
    }

    #[test]
    fn sanitize_keeps_valid_values() {
        let p = DockPrefs { cx: 0.25, edge: "left".into(), theme: "custom".into(),
            bg: "#AbCdEf".into(), accent: "#00ff00".into(), opacity: 0.6, ..DockPrefs::default() };
        assert_eq!(p.clone().sanitize(), p);
    }
}
