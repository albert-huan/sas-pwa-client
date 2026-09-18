# SAS PWA Client

A lightweight **Tauri v2 desktop client** for **SAS Viya / SAS Studio**.

It opens your SAS site inside a dedicated WebView2 window and keeps the session alive
(no idle timeout, no reliance on Edge/PWA installation), while letting the user configure
the site address instead of hard-coding it into the binary.

> Chinese documentation: [README_ZH.md](./README_ZH.md)

---

## What it is

This is **not** a PWA. It is a native Tauri shell (Rust + WebView2) that loads a remote
SAS Viya site. The real PWA is the remote SAS site itself; this client only injects scripts
to fake the PWA detection signals so the SAS front end takes the standalone branch and stops
timing out the session.

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
- **Session keep-alive** — disables WebView2 background throttling + spoofs PWA
  (`display-mode` / `navigator.standalone`) + periodic `mousemove` pulse, so SAS never idles out.
- **Tray resident** — closing a window only hides it to the tray; the session keeps running.
- **Frameless mode** — optional injected title bar with drag region, minimize and hide buttons.
- **Single instance** — a second launch just focuses the running instance.
- **DevTools** — press F12 in release builds too (`devtools` feature enabled).
- **Encrypted credentials (DPAPI)** — optionally store username/password on Windows via DPAPI;
  the password is never sent back to the front end.

---

## Requirements

- **Windows** with the WebView2 Runtime (Edge Chromium).
- **Node.js 18+** and npm (builds the settings page only).
- **Rust toolchain** + [Tauri v2 prerequisites](https://tauri.app/start/prerequisites/).

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
  "last_site_id": "prod"
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
  capabilities/default.json   Minimal IPC permissions for settings + site-* windows
  icons/                      App icons
dist/                         Built frontend (loaded as the Settings window)
```

### IPC commands used by the Settings page

| Command                | Purpose                                                                                   |
| ---------------------- | ----------------------------------------------------------------------------------------- |
| `get_config`         | `{ sites, last_site_id, config_path, version }`                                         |
| `save_config(sites)` | Validate → persist → refresh tray → apply frameless changes (returns normalized sites) |
| `open_site(id)`      | Open / focus the SAS window (`id` omitted → last used environment)                     |
| `hide_settings`      | Hide the Settings window                                                                  |
| `get_credential`     | Returns username / whether a password is stored (password is never exposed)               |
| `save_credential`    | Encrypt and store username / password for a site                                          |
| `clear_credential`   | Remove stored credentials for a site                                                      |

---

## Notes & limitations

- Keep-alive simulates user activity and makes the page believe it runs as a PWA; actual effect
  depends on the SAS front end. Server-side `enablesPWATimeout=false` makes it airtight.
- Changing the URL of an already open environment does not reload it immediately — the shell only
  navigates when scheme/host/port differ (sessions are not dropped on every click).
- `src-tauri/capabilities/default.json` grants core window permissions to `settings` and
  `site-*` windows; tighten it if you consider the loaded site untrusted.
- Credential encryption uses Windows DPAPI and is bound to the current user + machine; it is not
  portable across machines or users.

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
