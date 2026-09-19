# SAS PWA 客户端

一个轻量的 **Tauri v2 桌面客户端**，用于 **SAS Viya / SAS Studio**。

它把 SAS 站点放进独立的 WebView2 窗口打开，并保持会话在线（不因空闲超时掉线，也不依赖 Edge /
PWA 安装）；同时站点地址可由用户配置，而不是写死在二进制里。

> English version: [README.md](./README.md)

---

## 这是什么

本项目**不是** PWA，而是一个原生 Tauri 壳（Windows 用 WebView2，Linux 用系统的 WebKitGTK）去加载
远程 SAS Viya 站点。真正的 PWA 是远程 SAS 站点本身；本客户端只是注入脚本伪造 PWA 判定信号，让 SAS
前端走 standalone 分支、停止对会话计时超时。

关键设计：

- **站点地址可配置，不写死**。不同公司的 Viya 域名不同，地址在「设置」窗口填写一次并存到本地；
  多个环境（生产 / 测试 / 开发）可并存。
- **定位仍是 SAS 专用客户端**。虽然是加载远程地址，但这是为 SAS Studio 行为（保活、无边框条等）
  定制的，并非「打开任意网页」的通用浏览器。

---

## 功能

- **人工填写站点地址**：在「设置」窗口填写 Viya 地址（只填域名会自动补 `https://`），点
  「保存并打开」。
- **配置可保存**：所有环境持久化到 `config.json`，随时增删改。
- **默认登录环境**：把某个环境设为默认后，下次启动直接连接它，不再先显示设置页；没设默认时才
  启动进设置页供选择。列表里「设为默认 / 取消默认」可随时切换，托盘菜单给默认项标注「（默认）」。
- **会话不掉线**：关闭 WebView2 后台节流（仅 Windows）+ PWA 伪装（`display-mode` /
  `navigator.standalone`）+ 周期性 `mousemove` 脉冲，让 SAS 不再空闲超时。
- **托盘常驻**：关闭窗口只是隐藏到托盘，会话继续存活。
- **无边框模式**：可选自绘标题条（可拖动、最小化、隐藏到托盘）。
- **单实例**：再次启动只会聚焦已运行的实例。
- **DevTools**：release 版也能按 F12（已开启 `devtools` feature）。
- **凭据加密保存（DPAPI）**：可选在 Windows 上用 DPAPI 保存用户名 / 密码，密码永不下发前端。

---

## 环境要求

- **Windows** + WebView2 Runtime（Edge Chromium）。
- **Node.js 20.19+（或 22.12+）** 与 npm（仅用于构建设置页；Vite 8 / rolldown 需要该版本）。
- **Rust 工具链**及 [Tauri v2 前置依赖](https://tauri.app/start/prerequisites/)。
- **Linux（可选）**：直接用发布好的 `.deb`，见下节。

---

## Linux（.deb）

发布包在 **Ubuntu 22.04** 上构建，兼容 22.04 / 24.04 / 26.04：

```bash
sudo apt install "./SAS PWA Client_x.y.z_amd64.deb"   # 用 apt，别用 dpkg -i（依赖/推荐包不会自动装）
```

安装时会一并装上中文字体（`fonts-noto-cjk` / `fonts-wqy-microhei` / `fonts-arphic-uming` 任一，列为
Recommends）。若安装时用了 `--no-install-recommends`、或系统里没有任何中文字体，界面中文会显示成方块：

```bash
sudo apt install fonts-noto-cjk
```

**界面卡顿 / 闪烁**：WebKitGTK 的加速合成只走 DMA-BUF —— 关掉它就等于退回 CPU 画图，滚动和动画会明显变卡。
所以默认保持 WebKit 自己的路径，只在已知有问题的环境（NVIDIA 专有驱动、WSLg）自动降级。设置页底部的
**「Linux 渲染」**区块可三选一（改完需重启客户端）：

| 模式 | 实际动作 | 什么时候用 |
| --- | --- | --- |
| 自动（默认） | 检测到 WSL / NVIDIA 时关 DMA-BUF；**没有 GPU 加速**（`/dev/dri` 里没有 `renderD*` 渲染节点）时再关掉加速合成，走纯 CPU 合成 | 一般情况 |
| 流畅优先 | 保持 DMA-BUF，并强制开启加速合成（`WEBKIT_FORCE_COMPOSITING_MODE=1`） | 界面正常但觉得不流畅 |
| 兼容优先 | 关闭 DMA-BUF（`WEBKIT_DISABLE_DMABUF_RENDERER=1`） | 花屏 / 闪烁 / 白屏 |

**没有 GPU 加速的机器**只能软件渲染：服务器主板 BMC 上的 ASPEED / Matrox 这类 2D 显示芯片（日志里会出现
`card1:ASPEED(BMC)/ast` 且**没有** `renderD*` 节点）也属于这一类，另外还有没开 3D 的虚拟机、没映射 `/dev/dri`
的容器。这类机器上 `WEBKIT_SHOW_FPS=1 sas-pwa-client` 会在页面右上角显示实时帧率，方便对照上面几种模式的
效果；但流畅度的上限由 CPU 决定——确认方式就是看启动日志里的 `linux:` 行（出现「无 renderD* 渲染节点」或
`GPU=llvmpipe` 即属此列），客户端侧已经做了能做的，要顺滑得换到有显卡的机器（或让 X 会话用上显卡）。

也可以不改配置，用环境变量临时试（环境变量优先级最高）：

```bash
WEBKIT_FORCE_COMPOSITING_MODE=1 sas-pwa-client    # 相当于「流畅优先」
WEBKIT_DISABLE_DMABUF_RENDERER=1 sas-pwa-client   # 相当于「兼容优先」
WEBKIT_DISABLE_COMPOSITING_MODE=1 sas-pwa-client  # 再退一步：完全关闭合成
```

启动日志 `~/.config/com.saspwa/debug.log` 里的 `linux: ...` 一行会记录：生效的渲染模式与原因、会话类型
（x11 / wayland）、GPU（`/sys/class/drm`，如 `card0:Intel`）、显示设备节点、SSH 会话、GL 相关环境变量、
中文字体。排查问题时直接贴这一行即可 —— 例如显示「无 /dev/dri」就说明当前只能用 CPU 软件渲染
（llvmpipe），这时换渲染模式帮助有限，优先把显卡驱动 / 设备映射搞定。

**切到别的程序再切回来，页面重新刷新一遍**：Linux 下页面由 WebKitGTK 渲染，窗口被最小化 / 隐藏（含本
客户端「关闭 = 隐藏到托盘」）时 WebKit 会把页面标成 hidden 并派发 `visibilitychange`，SAS 前端收到后
常会重新拉数据、把表格从第一行重绘一遍。Windows 侧靠 WebView2 的 `--disable-backgrounding-occluded-windows`
关掉了同类行为；Linux 侧由注入脚本做等价伪装（开启「保持会话不超时」时生效：恒报 `visible` 并丢弃
`visibilitychange`）。另外若桌面**没有合成器**（很多轻量桌面默认不开，如 XFCE / LXDE / 无 `picom`），X11 在
窗口被遮挡后会丢弃窗口内容，切回来必然整窗重绘 —— 这时开合成器（`picom`、`xfwm4 --composer=on`，
或换 GNOME / KDE / Wayland 会话）比换渲染模式有用得多。

---

## 构建 / 运行

```bash
npm install
npm run tauri dev      # 开发模式：vite 监听 :1420 + Rust 壳
npm run tauri build    # 构建：dist/ 前端 + msi/nsis 安装包
```

> **直接用 cargo 构建必须启用 `custom-protocol`**：Tauri 用该 feature 区分「生产 / dev」
> （`npm run tauri build` 由 Tauri CLI 自动加上）。若直接编译 release 而不加该 feature，
> `cfg(dev)` 仍为真，设置窗口会去加载 `build.devUrl`（`http://localhost:1420`），
> 出现 `ERR_CONNECTION_REFUSED`：
>
> ```bash
> cd src-tauri
> cargo build --release --features custom-protocol   # 产物：target/release/sas-pwa-client.exe
> ```
>
> `build.rs` 会在 release 构建缺少该 feature 时打印告警。

---

## 使用方式

1. **启动行为**：没配置过环境、或没设置默认登录环境 → 自动打开「设置」窗口；已设置默认环境 →
   直接连接该环境（要改配置从托盘「站点设置」进入）。
2. 填写**名称**（如「生产环境」）与 **SAS 站点地址**（`https://viya.<公司域名>/`），点击
   「保存并打开」。
3. 站点在自己的窗口中打开，托盘菜单同步出现「打开 <名称>」。
4. 托盘菜单：每个环境一个「打开 <名称>」·「站点设置」·「重新加载当前页面」·「显示全部窗口」·
   「退出」。
5. 托盘**左键**切换最近使用窗口的显隐；**关闭窗口**只会隐藏到托盘（SAS 会话不断），真正退出请用
   托盘「退出」。

### 站点级选项

| 选项                       | 含义                                                                               |
| -------------------------- | ---------------------------------------------------------------------------------- |
| 保持会话不超时（伪装 PWA） | 注入 PWA 伪装，让 SAS 走 standalone 分支；关闭后恢复普通浏览器分支，空闲超时生效。 |
| 活动脉冲间隔（秒）         | 每隔 N 秒派发一次合成`mousemove` 重置 SAS 空闲计时器，`0` 关闭，默认 `120`。 |
| 无边框窗口                 | 去掉系统标题栏，改用注入的自绘标题条。                                             |
| 设为默认登录环境           | 下次启动自动连接该环境；同时只允许一个默认环境。                                   |

---

## 配置文件

保存在 Tauri 的应用配置目录，Windows 下为
`%APPDATA%\sas-pwa-client\config.json`（目录名与程序 exe 同名，而非 bundle identifier；设置页
底部会显示具体路径）。示例：

```json
{
  "sites": [
    {
      "id": "prod",
      "name": "生产环境",
      "url": "https://viya.example.com/",
      "keep_awake": true,
      "pulse_seconds": 120,
      "frameless": false,
      "default": true
    }
  ],
  "last_site_id": "prod"
}
```

---

## 目录结构

```
src/                          设置页 UI（原生 HTML/JS，无需框架）
src-tauri/
  src/main.rs                 程序入口
  src/lib.rs                 壳逻辑：窗口、托盘、命令、注入脚本
  src/config.rs              配置模型、校验、持久化
  src/credentials.rs         加密凭据存储（Windows DPAPI）
  capabilities/default.json  settings 与 site-* 窗口的最小 IPC 权限
  icons/                     应用图标
dist/                        构建后的前端（作为设置窗口加载）
```

### 设置页使用的 IPC 命令

| 命令                   | 作用                                                                 |
| ---------------------- | -------------------------------------------------------------------- |
| `get_config`         | 返回`{ sites, last_site_id, config_path, version }`                |
| `save_config(sites)` | 校验 → 落盘 → 刷新托盘 → 同步无边框外观（返回归一化后的站点数组） |
| `open_site(id)`      | 打开/聚焦站点窗口（省略`id` 则打开最近使用的环境）                 |
| `hide_settings`      | 隐藏设置窗口                                                         |
| `get_credential`     | 返回用户名 / 是否已存密码（密码永不下发）                            |
| `save_credential`    | 加密保存某站点的用户名 / 密码                                        |
| `clear_credential`   | 删除某站点的已存凭据                                                 |

---

## 已知限制

- 保活依赖「模拟用户活动 + 让页面判定为 PWA」，具体是否生效取决于 SAS 前端实现；服务端把
  `enablesPWATimeout` 设为 `false` 时最稳妥。
- 修改已打开环境的地址不会立刻重载窗口——只有当 scheme/host/port 变化时才重新导航，避免每次点击
  都丢会话。
- `src-tauri/capabilities/default.json` 向 `settings` 与 `site-*` 窗口开放了核心窗口权限，若认为
  所加载站点不可信，请按需收紧。
- 凭据加密使用 Windows DPAPI，绑定「当前用户 + 本机」，无法跨机器 / 跨用户迁移。

---

## 多语种

设置页支持**中文**与**英文**。首次打开按系统 / 浏览器语言自动选择，也可随时通过右上角下拉框
切换；选择会本地记忆（存于 `localStorage`）。

- `跟随系统` —— 按系统 / 浏览器语言自动判定。
- `中文` / `English` —— 强制指定某种语言。

如需新增语言，在 `src/i18n.js` 的 `dict` 里补一份词典并加入可选项即可，界面与切换器会自动适配。

---

## 免责声明

本项目为**非 SAS 官方应用**，开发者无意冒充或代表 SAS 官方，仅为提升日常工作便利性而开发。

联系方式：albert.huan@outlook.com
