//! `core/` holds application logic. `AGENTS.md` invariant 1: this module and
//! everything under it never imports `tauri`. `ipc/` (not yet created) is the
//! only thing allowed to depend on both `core/` and `tauri`.

pub mod types;
