//! `plugin-bashkit` - the bashkit Plugin, built against contract 0.4.
//! Today it loads, describes itself as `bashkit` at its crate version,
//! and unloads. It inserts no Pack yet.
//! `cargo build -p plugin-bashkit --release` writes `plugin_bashkit.dll`
//! on Windows, `libplugin_bashkit.so` on Linux, and
//! `libplugin_bashkit.dylib` on macOS to `target/release`; that one file
//! is what goes in a Host's Plugin directory.
//!
//! ## Invariants
//!
//! - Uses `runvil-plugin-sdk` alone and writes no `unsafe`.
//! - Ships only as a dynamic library, so it exports `runvil_plugin`
//!   unconditionally.

use runvil_plugin_sdk::{Plugin, Refusal};

/// The bashkit Plugin's instance, with no state yet. It only describes
/// itself: it names no Pack, so the Host asks it for none.
#[derive(Debug)]
pub struct Bashkit;

impl Plugin for Bashkit {
    /// One Pack id segment.
    const NAME: &'static str = "bashkit";
    /// This crate's version.
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Loads with no state and never refuses.
    fn load() -> Result<Bashkit, Refusal> {
        Ok(Bashkit)
    }
}

// Exports the C entry point `runvil_plugin`.
runvil_plugin_sdk::export_plugin!(Bashkit);

#[cfg(test)]
mod tests {
    use runvil_plugin_sdk::sys::RV_OK;
    use runvil_plugin_sdk::testing;

    use super::Bashkit;

    #[test]
    fn the_plugin_loads_as_bashkit_at_its_version_with_no_pack_and_unloads() {
        let loaded = testing::load::<Bashkit>().unwrap();
        assert_eq!(loaded.name(), "bashkit");
        assert_eq!(loaded.version(), "0.1.0");
        assert_eq!(loaded.packs().len(), 0);
        assert_eq!(loaded.unload(), RV_OK);
    }
}
