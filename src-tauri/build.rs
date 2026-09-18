fn main() {
    // 生产构建必须启用 `custom-protocol`（Tauri CLI 会自动加）。
    // 缺少它时 Tauri 的 `cfg(dev)` 为真，设置窗口会加载 `build.devUrl`（localhost:1420）→
    // ERR_CONNECTION_REFUSED。这里只给告警（不 panic），避免影响 `tauri build`。
    if std::env::var("PROFILE").as_deref() == Ok("release")
        && std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_none()
    {
        println!(
            "cargo:warning=release 构建未启用 custom-protocol：请改用 `npm run tauri build` \
             或 `cargo build --release --features custom-protocol`，否则运行时会按 dev 模式 \
             去加载 localhost:1420。"
        );
    }

    // 应用级 ACL 清单：站点窗口承载的是**远程来源**（https://<客户域名>/），
    // Tauri 对 remote origin 的 IPC 会先查 ACL，没有匹配的 capability + permission
    // 就一律拒绝执行（连自定义命令也不例外），表现为「F11 无反应、自绘标题条按钮点不动」。
    // 这里声明全部自定义命令，权限标识为 `allow-<命令名>`（下划线换成连字符），
    // 再由 capabilities/*.json 按窗口与来源分别授权：本地设置页给全量，远程页面只给最小集。
    let attrs = tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_config",
            "save_config",
            "open_site",
            "hide_settings",
            "toggle_frameless",
            "show_settings",
            "get_credential",
            "save_credential",
            "clear_credential",
        ]),
    );
    if let Err(e) = tauri_build::try_build(attrs) {
        panic!("tauri build script failed: {e}");
    }
}
