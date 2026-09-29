# amd-tauri — a spike, not a shipped interface

Ticket 078 in `agile-md-tickets` asked whether the desktop board could be
rebuilt on [Tauri](https://tauri.app) instead of
egui/eframe. This is that spike: a **read-only** board view (columns,
cards, id/title/points/assignee/tags) reading through `amd-lib` exactly
the way `amd-cli`/`amd-ui` already do — no drag-and-drop, no ticket
editor, none of the rich-text/field work from tickets 068/069/071/074
re-implemented. See the ticket for the findings and the keep-or-park
recommendation.

## Why this isn't a normal workspace member

`src-tauri/Cargo.toml` declares its own empty `[workspace]` table on
purpose, so this crate has its own `Cargo.lock` and target directory
instead of joining the root workspace. A spike shouldn't grow the main
project's dependency graph (Tauri/webkit2gtk's own tree is large) or
touch its `Cargo.lock` until there's an actual decision to keep this.

## Running it

Needs the Tauri Linux prerequisites installed (not previously on this
project's dev setup — see the ticket): `pkg-config`,
`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`,
`librsvg2-dev`, and the `tauri-cli` cargo binary
(`cargo install tauri-cli --version "^2.0"`).

```sh
cd crates/amd-tauri/src-tauri
GDK_BACKEND=x11 cargo tauri dev   # GDK_BACKEND=x11 needed under Xwayland —
                                   # GTK has its own backend selection,
                                   # separate from winit's
```

The frontend (`../dist/`) is plain HTML/CSS/vanilla JS — no framework, no
`bun`/npm build step, on purpose (`withGlobalTauri` in `tauri.conf.json`
injects `window.__TAURI__` so `invoke()` needs no `@tauri-apps/api`
package either). That was a deliberate spike-scoping choice, not a verdict
on what a real Tauri frontend should look like if this goes further.
