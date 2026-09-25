//! Nap Wallet on a phone: the wallet's own pages (apps/nap-web, built with
//! `--mode mobile`) in the platform's web view — the wallet on nap.cavebro.win:
//! keys made and sealed on the device, chain reads and signed frames to Nap's
//! server. One native piece is exposed to the pages: `napvault`, which keeps
//! the kept wallet's password behind Face ID / fingerprint (§151).

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_napvault::init())
        .run(tauri::generate_context!())
        .expect("Nap Wallet could not start");
}
