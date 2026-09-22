# SAS PWA Client

A lightweight **Tauri v2 desktop client** for **SAS Viya / SAS Studio**.

It opens your SAS site inside a dedicated WebView2 window and keeps the session alive
(no idle timeout, no reliance on Edge/PWA installation), while letting the user configure
the site address instead of hard-coding it into the binary.

> Chinese documentation: [README_ZH.md](./README_ZH.md)

---

## What it is

This is **not** a PWA. It is a native Tauri shell (WebView2 on Windows, the system's WebKitGTK on
Linux) that loads a remote SAS Viya site. The real PWA is the remote SAS site itself; this client
only injects scripts to fake the PWA detection signals so the SAS front end takes the standalone
branch and stops timing out the session.

Key design decisions:

- **Site address is configurable, not hardcoded.** Every company has its own Viya domain, so
  the URL is entered once in a Settings window and stored locally. Multiple environments
  (prod / test / dev) can coexist.
- **It stays SAS-specific.** Despite loading a remote URL, this is not a generic "open any
  website" browser — it is tailored to SAS Studio behaviour (keep-alive, frameless bar, etc.).

---

## Features

- **Configurable site address** — type the Viya URL in the Settings window (a bare host is
  auto-prefixed with `https://`) and press *Save and open*.
- **Persisted configuration** — all environments are saved to `config.json`; add / edit /
  delete any time.
- **Default sign-in environment** — mark one environment as default and the client connects to
  it automatically on launch (no Settings window). Without a default, the Settings window opens
  first so you can pick one. Toggle anytime from the list; the tray menu annotates the default
  with *(default)*.
- **Session keep-alive** — disables WebView2 background throttling (Windows only) + spoofs PWA
  (`display-mode` / `navigator.standalone`) + periodic `mousemove` pulse, so SAS never idles out.
- **Tray resident** — closing a window only hides it to the tray; the session keeps running.
- **Frameless mode** — optional injected title bar with drag region, minimize and hide buttons.
- **Single instance** — a second launch just focuses the running instance.
- **Update check** — the `Check for updates` button in the Settings window compares against the
  latest GitHub release and opens the download page in your browser.
- **DevTools (off by default)** — site windows load a remote SAS page, so having DevTools around
  means the credentials injected for auto-fill are exposed. Enable it explicitly under
  *Advanced* in the Settings window (applies to windows opened afterwards).
- **Credential storage** — DPAPI-encrypted on Windows (bound to the current account + machine);
  **other platforms have no DPAPI and only hex-encode the password, i.e. effectively plaintext**,
  as the Settings window states. The password is never sent back to the front end.

---

## Requirements

- **Windows** with the WebView2 Runtime (Edge Chromium).
- **Node.js 20.19+ (or 22.12+)** and npm (builds the settings page only; Vite 8 / rolldown requires it).
- **Rust toolchain** + [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/).
- **Linux (optional)** — use the published `.deb`, see the next section.

---

## Linux (.deb)

The published package is built on **Ubuntu 22.04** and runs on 22.04 / 24.04 / 26.04:

```bash
sudo apt install "./SAS PWA Client_x.y.z_amd64.deb"   # use apt, not dpkg -i (deps + recommends)
```

A CJK font (`fonts-noto-cjk` / `fonts-wqy-microhei` / `fonts-arphic-uming`, whichever is available) is
installed as a *Recommends*. With `--no-install-recommends`, or on a system with no Chinese font at all,
Chinese text renders as boxes:

```bash
sudo apt install fonts-noto-cjk
```

**Laggy / flickering UI**: WebKitGTK's accelerated compositing only goes through DMA-BUF — disabling it means
falling back to CPU painting, which makes scrolling and animations visibly slower. So the WebKit default is
kept, and only environments known to be broken (NVIDIA proprietary driver, WSLg) are downgraded automatically.
The **"Linux rendering"** section at the bottom of the settings page offers three modes (restart required):

| Mode | What it does | When to use |
| --- | --- | --- |
| Automatic (default) | Disables DMA-BUF when WSL / NVIDIA is detected; on machines without GPU acceleration (no `renderD*` render node under `/dev/dri`) it also turns off accelerated compositing and composites on the CPU | Normal case |
| Performance first | Keeps DMA-BUF and forces accelerated compositing (`WEBKIT_FORCE_COMPOSITING_MODE=1`) | Display is fine but feels slow |
| Compatibility first | Disables DMA-BUF (`WEBKIT_DISABLE_DMABUF_RENDERER=1`) | Flicker / artifacts / blank window |

**Machines without GPU acceleration** can only software-render: that includes the 2D BMC chips found on server
mainboards (the log then shows e.g. `card1:ASPEED(BMC)/ast` with **no** `renderD*` node), VMs without 3D acceleration
and containers without `/dev/dri` mapped in. On such machines `WEBKIT_SHOW_FPS=1 sas-pwa-client` shows a live FPS box
in the top-right corner of the page, handy for comparing the modes above; but the smoothness ceiling is the CPU. The
`linux:` line in the start-up log tells you which case you are in (“no renderD* render node” or `GPU=llvmpipe`). The
client already does what it can; for a smooth experience run it on a machine with a GPU (or make the X session use
one).

You can also try the routes without touching the config (env vars take precedence):

```bash
WEBKIT_FORCE_COMPOSITING_MODE=1 sas-pwa-client    # equals "Performance first"
WEBKIT_DISABLE_DMABUF_RENDERER=1 sas-pwa-client   # equals "Compatibility first"
WEBKIT_DISABLE_COMPOSITING_MODE=1 sas-pwa-client  # one step further: disable compositing
```

The `linux: ...` line in `~/.config/com.saspwa/debug.log` records the effective render mode and why, the session
type (x11 / wayland), the GPU (`/sys/class/drm`, e.g. `card0:Intel`), DRM device nodes, SSH sessions, GL-related
environment variables and the CJK font found. Paste that single line when reporting a problem — if it says there
is no `/dev/dri`, WebKit is stuck with CPU (llvmpipe) rendering, and changing the render mode will not help much.

**The page re-renders from the first row after switching to another app**: on Linux the page is rendered by
WebKitGTK. When the window is minimised/hidden (including this client's "close = hide to tray"), WebKit marks the
page hidden and fires `visibilitychange`; the SAS front-end then often re-fetches data and repaints its table from
row one. On Windows that class of behaviour is disabled through WebView2's
`--disable-backgrounding-occluded-windows`; on Linux the injected script does the equivalent (with "keep session
alive" enabled: always report `visible` and drop `visibilitychange`). If your desktop has **no compositor**
(common on lightweight desktops such as XFCE / LXDE, or without `picom`), X11 throws away the window contents once
it is obscured, so a full repaint on return is unavoidable — enabling a compositor (`picom`, `xfwm4 --composer=on`,
or a GNOME / KDE / Wayland session) helps far more than switching render modes.

**No tray icon, no way to quit**: the tray relies on a desktop StatusNotifier host, which some environments
(WSLg, trimmed-down X11 sessions) do not provide — it simply never shows up, and since the tray's *Quit* is the
only quit entry point you end up killing the process. Use the command-line switches instead:

```bash
sas-pwa-client --hide    # tuck every window away (same as "hide to tray" on each)
sas-pwa-client --quit    # quit the running instance
```

---

## Build & run

```bash
npm install
npm run tauri dev      # dev: vite on :1420 + Rust shell
npm run tauri build    # build: dist/ frontend + msi/nsis installers
```

> **Building with plain `cargo` requires the `custom-protocol` feature.** Tauri decides
> "production vs dev" from that feature (the Tauri CLI sets it automatically for
> `tauri build`). If you build the release binary directly without it, the Settings window
> loads `build.devUrl` (`http://localhost:1420`) and shows `ERR_CONNECTION_REFUSED`:
>
> ```bash
> cd src-tauri
> cargo build --release --features custom-protocol   # exe: target/release/sas-pwa-client.exe
> ```
>
> `build.rs` warns when a release build is missing this feature.

---

## Usage

1. **Launch behaviour** — no environment configured (or none set as default) → the Settings
   window opens automatically; a default is set → that environment opens directly (change
   anything from tray *Site settings*).
2. Enter a **name** (e.g. `Prod`) and the **SAS site address** (`https://viya.<your-domain>/`),
   then press **Save and open**.
3. The site opens in its own window; a tray entry `Open <name>` appears per environment.
4. Tray menu: one `Open <name>` per environment · `Site settings` · `Reload` · `Show all` · `Quit`.
5. Tray **left click** toggles the last active window; **closing** a window hides it to the tray
   (session survives). Use `Quit` to quit for real.
6. **Command-line switches** (handy on Linux where the tray may be unavailable; also usable in
   scripts / shortcuts on Windows):
   - `sas-pwa-client --hide` — tuck every window away (same as clicking "hide to tray" on each);
   - `sas-pwa-client --quit` — quit the running instance (if none is running, it just exits).
   When an instance is already running, the new process only forwards the action.

### Per-site options

| Option                 | Meaning                                                                                                       |
| ---------------------- | ------------------------------------------------------------------------------------------------------------- |
| `Keep session alive` | Injects the PWA spoof so SAS takes the standalone branch. Off → normal browser branch + idle timeout.        |
| `Activity pulse`     | Seconds between synthetic`mousemove` events that reset the SAS idle timer. `0` disables, default `120`. |
| `Frameless`          | Removes the OS title bar, shows the injected drag bar instead.                                                |
| `Default`            | Connect automatically on next launch; only one environment can be the default.                                |

---

## Configuration file

Stored under the app config directory — on Windows:
`%APPDATA%\sas-pwa-client\config.json` (folder named after the program, not the bundle
identifier; the exact path is shown at the bottom of the Settings window). Example:

```json
{
  "sites": [
    {
      "id": "prod",
      "name": "Production",
      "url": "https://viya.example.com/",
      "keep_awake": true,
      "pulse_seconds": 120,
      "frameless": false,
      "default": true
    }
  ],
  "last_site_id": "prod",
  "ui_theme": "system",
  "render_mode": "auto",
  "dev_tools": false
}
```

---

## Project layout

```
src/                          Settings UI (plain HTML/JS, no framework)
src-tauri/
  src/main.rs                 Entry point
  src/lib.rs                  Shell logic: windows, tray, commands, injected script
  src/config.rs               Config model, validation, persistence
  src/credentials.rs          Encrypted credential storage (Windows DPAPI)
  capabilities/default.json   IPC permissions for the local Settings window
                              (remote site pages use remote-sites.json)
  icons/                      App icons
dist/                         Built frontend (loaded as the Settings window)
```

### IPC commands used by the Settings page

| Command                | Purpose                                                                                   |
| ---------------------- | ----------------------------------------------------------------------------------------- |
| `get_config`         | `{ sites, last_site_id, ui_theme, render_mode, dev_tools, platform, config_path, version }` |
| `save_config(sites)` | Validate → persist → refresh tray → sync frameless state, destroy windows of deleted sites, navigate windows whose URL changed |
| `open_site(id)`      | Open / focus the SAS window (`id` omitted → last used environment)                     |
| `new_site_window(id)` | Open another window for the same site (shares cookies / login state)                    |
| `close_window`       | Really close the current site window (destroys the WebView, not hide-to-tray)           |
| `close_site_windows` | Really close every window of a site; returns how many were closed                       |
| `site_window_counts` | Number of open windows per site                                                          |
| `toggle_frameless`   | Toggle frameless / decorated for the current site window                                |
| `set_ui_theme`       | Store the theme preference (system / light / dark)                                      |
| `set_render_mode`    | Store the Linux render mode (takes effect after a restart)                              |
| `set_dev_tools`      | Allow opening DevTools (off by default; applies to newly opened windows)                |
| `open_url(url)`      | Open a link in the system browser (GitHub domains only; used by "Check for updates")    |
| `show_settings`      | Open the site-settings window                                                           |
| `hide_settings`      | Hide the Settings window                                                                  |
| `get_credential`     | Returns username / whether a password is stored (password is never exposed)               |
| `save_credential`    | Store username / password for a site                                                      |
| `clear_credential`   | Remove stored credentials for a site                                                      |

---

## Notes & limitations

- Keep-alive simulates user activity and makes the page believe it runs as a PWA; actual effect
  depends on the SAS front end. Server-side `enablesPWATimeout=false` makes it airtight.
- **Changing the keep-alive switch or pulse interval requires closing and reopening that site's
  window**: the values are baked into the injected script when the window is created, and later
  config saves do not re-inject (the Settings window tells you so on save). This is deliberate —
  the visibility / `matchMedia` spoofs are installed once and cannot be undone, so a hot update
  would give the illusion of being only half applied.
- A window is only re-navigated when **no navigation ever happened** (still `about:blank`); when a
  site redirects to an external IdP over SSO the shell does not pull the address back, so an
  in-progress login is not wiped. Changing a site's URL does navigate its open windows.
- `src-tauri/capabilities/default.json` only covers the local Settings window; remote site pages
  (`https://`) go through the much smaller `remote-sites.json`. Tighten that one if you consider
  the loaded site untrusted.
- Credential storage: DPAPI on Windows (bound to the current user + machine, not portable);
  **other platforms have no DPAPI — the password is only hex-encoded, i.e. effectively
  plaintext** — keep `credentials.json` unreadable for other users yourself.
- DevTools is off by default (enable under *Advanced* in the Settings window; applies to newly
  opened windows): site windows load a remote page, so enabling it exposes the injected
  credentials.
- **There is no self-download/install updater**: "Check for updates" only compares against the
  latest GitHub release and opens the download page in your browser; you still replace the binary
  or reinstall the .deb manually.

---

## Localization

The Settings window supports **Chinese** and **English**. On first launch the language is
auto-detected from the OS / browser locale; it can be switched anytime from the dropdown in the
top-right corner, and the choice is remembered locally (stored in `localStorage`).

- `跟随系统 / Follow system` — pick the language from the system/browser locale automatically.
- `中文` / `English` — force a specific language.

To add another language, extend the `dict` in `src/i18n.js` and add its code to the choice list —
the UI and switcher adapt automatically.

---

## Disclaimer

This project is an **unofficial, non-SAS application**. The developer does not intend to impersonate
or represent SAS Institute Inc. in any way; it was built solely to improve day-to-day convenience.

Contact: albert.huan@outlook.com
