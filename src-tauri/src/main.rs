#![windows_subsystem = "windows"]

// 程序入口，转发给 lib::run()。
fn main() {
    sas_pwa_client_lib::run();
}
