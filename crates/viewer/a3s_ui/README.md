# a3s_ui

Theme, icons and reusable egui widgets for the Arma3Sync viewer.

- `theme`: light/dark tokens with macOS system colours and their egui mapping (default: dark).
- `fonts`: the embedded Geist faces (SIL OFL 1.1, see `assets/fonts/Geist-OFL.txt`).
- `icon`: the Phosphor icon font (MIT; icons are text glyphs, `icon::icons::PLAY`) and the
  application icon PNG.
- `widgets`: macOS form controls (form row, toggle switch, segmented control, pop-up button),
  sidebar rows, status pill and step rows, thin progress bar, tri-state check tree (with an
  optional status pill per row), check list, button helpers.
- `frames`: grouped box, unified toolbar, sidebar, status bar and pane divider.
- `dialog`: the OK/Cancel and progress dialog bases.
- `gallery`: shows everything; run it with `cargo run -p a3s_ui --example demo`.

Call `a3s_ui::install(ctx)` once before the first frame.

Snapshot tests render headless through wgpu (lavapipe on Linux); regenerate them with
`UPDATE_SNAPSHOTS=1`.
