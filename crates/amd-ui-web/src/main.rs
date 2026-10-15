// The UI only runs as WebAssembly inside the Tauri window. Natively it still
// compiles (so `cargo clippy --workspace` checks it) but is never mounted.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

mod admin;
mod api;
mod app;
mod board;
mod dialog;
mod documents;
mod editor;
mod home;
mod ideas;
mod panel;
mod preferences;
mod rich_dom;
mod rich_editor;
mod roadmap;
mod term_view;
mod tree;

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    // A panic stops the whole page; say so where it can be seen, on amdui's
    // own error output, as well as in the web view's console.
    let console = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        api::report_panic(&info.to_string());
        console(info);
    }));
    leptos::mount::mount_to_body(app::App);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "amd-ui-web is the desktop board's WebAssembly front end and cannot run on its own.\n\
         Build it with `trunk build` and run `amdui`, or develop with `cargo tauri dev` in crates/amd-ui."
    );
    std::process::exit(2);
}
