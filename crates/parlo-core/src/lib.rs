//! Parlo core: all logic, no Tauri dependency.

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
