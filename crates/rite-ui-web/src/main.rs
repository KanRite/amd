// The UI only runs as WebAssembly inside the Tauri window. Natively it still
// compiles (so `cargo clippy --workspace` checks it) but is never mounted.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

mod admin;
mod api;
mod app;
mod board;
mod counts;
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
mod update_bar;

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    // A panic stops the whole page; say so where it can be seen, on riteui's
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
        "rite-ui-web is the desktop board's WebAssembly front end and cannot run on its own.\n\
         Build it with `trunk build` and run `riteui`, or develop with `cargo tauri dev` in crates/rite-ui."
    );
    std::process::exit(2);
}
