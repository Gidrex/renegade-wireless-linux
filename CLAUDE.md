# renegade-wireless-linux

Rust CLI (`renegade`) + egui GUI (`renegade-gui`) for the ARDOR GAMING Renegade Wireless mouse.
Architecture, protocol, memory map and RE notes: `docs/DEVELOPMENT.md`. Read it before touching `src/`.

- CLI and GUI must offer the same features. Every new feature: `config.rs`/`macros.rs` → `mouse.rs` → `main.rs` → GUI `Model`.
- Buttons are numbered 1–8 as on the OemDrv mouse diagram; `config::BUTTON_SLOTS` maps them to memory slots.
- User-facing text is Russian; code, comments and identifiers are English.
- QA gate: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- Hardware writes: back up first (`renegade backup`), restore and `cmp` after. The mouse sleeps within seconds, so ask the user to keep moving it during hardware tests.
- Keep `docs/DEVELOPMENT.md` and `README.md` in sync with what changes.
- Release: bump `Cargo.toml` version, push tag `vX.Y.Z`; `.github/workflows/release.yml` builds and publishes.
- Never commit `*.exe` (vendor installer) or `*.bin` (mouse memory dumps).
