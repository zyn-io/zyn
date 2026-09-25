//! Nap Wallet on a phone: the wallet's own pages (apps/nap-web, built with
//! `--mode mobile`) in the platform's web view. Nothing native is exposed to
//! the pages — no commands, no plugins — so the app is exactly the wallet on
//! nap.cavebro.win: keys made and sealed on the device, chain reads and signed
//! frames to Nap's server, and nothing else.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("Nap Wallet could not start");
}
