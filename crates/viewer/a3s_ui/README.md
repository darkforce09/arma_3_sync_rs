# a3s_ui

Theme, icons and reusable egui widgets for the Arma3Sync viewer.

- `theme`: light/dark tokens of the Java `ThemeLibrary` and their egui mapping (default: dark).
- `fonts`: the embedded Geist faces (SIL OFL 1.1, see `assets/fonts/Geist-OFL.txt`).
- `icon`: the 42 bundled icons with tone/state tinting and the application icon
  (see `assets/icons/README.md` for their origin).
- `widgets`: tri-state check tree, check list, labelled progress bar, button helpers.
- `frames`, `dialog`: `UiStyle` frames and the OK/Cancel and progress dialog bases.
- `gallery`: shows everything; run it with `cargo run -p a3s_ui --example demo`.

Call `a3s_ui::install(ctx)` once before the first frame.

Snapshot tests render headless through wgpu (lavapipe on Linux); regenerate them with
`UPDATE_SNAPSHOTS=1`.
