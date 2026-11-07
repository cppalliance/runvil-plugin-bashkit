---
name: bashkit plugin scaffold
overview: Set up runvil-plugin-bashkit as a workspace matching runvil-plugin-api, with a plugin-bashkit crate that loads through the contract 0.2 SDK and describes itself as bashkit. Pack registration waits until the contract can do it.
todos: []
isProject: false
---

<plan-context>

# The bashkit Plugin at Contract 0.2

The `runvil-plugin-bashkit` repository becomes a Rust workspace set up like `runvil-plugin-api`, holding one crate, `plugin-bashkit`, at `crates/plugin-bashkit`. It builds a dynamic library whose `runvil_plugin` entry point speaks contract 0.2 through `runvil-plugin-sdk`, and it loads and describes itself as `bashkit` at its crate version. It registers no Pack yet, because contract 0.2 has no way for a library Plugin to register one; a later plan adds the Pack once the contract can.

## Design

### Why it only describes itself

- Contract 0.2's `RvLoadArgs` holds only `refuse` and `describe_plugin`, and the Pack verbs 3 to 8 are reserved numbers with no constants (`runvil-plugin-api` `crates/runvil-plugin-sys/src/lib.rs` L63-81, L173-191).
- `runvil-plugin-sdk` 0.2.0 has no Pack API: its `Plugin` trait is `NAME`, `VERSION`, and `fn load() -> Result<Self, Refusal>`, and its entry point answers every other verb with `RV_ERR_UNSUPPORTED` (`runvil-plugin-api` `crates/runvil-plugin-sdk/src/lib.rs` L54-138, `src/entry.rs` L16-69).
- The Engine's loader leaves its `packs` parameter unused "until insert_pack exists" (`runvil` `crates/harness-internal/plugin-loader/src/plugin.rs` L40-44, L134-138).
- So the crate takes no dependency on the `bashkit` crate yet; nothing would use it until the Pack exists.

### The Plugin

- It follows `hello-plugin` (`runvil-plugin-api` `examples/hello-plugin/Cargo.toml` L1-25, `src/lib.rs` L11-29): a unit struct implementing `Plugin`, exported with `export_plugin!`.
- Unlike `hello-plugin`, the crate is a `cdylib` only, with no `rlib` crate type and no `dll` feature gating the export. Those exist so a Host can link a Plugin as a Rust library without a second unmangled `runvil_plugin` symbol, and this Plugin ships only as a dynamic library. A Host that later links it adds both back then.
- Its test is a unit test inside the crate, since a `cdylib`-only crate has no `rlib` for an integration test in `tests/` to link.
- `NAME` is `bashkit`, one Pack id segment (`runvil-plugin-api` `crates/runvil-plugin-sdk/src/names.rs` L8-10). The SDK fails the build on a bad name or version.
- `runvil-plugin-sdk` is `publish = false`, so it comes by git, pinned to the commit the Engine's loader already pins for `runvil-plugin-sys` (`runvil` `Cargo.toml`). `version = "0.2.0"` sits beside `git` and `rev`.
- When the workspace's `unsafe_code = "deny"` objects to the `export_plugin!` expansion, do what `hello-plugin` does.

### Repository setup

Copied from `runvil-plugin-api` (its `Cargo.toml` L1-50, `rust-toolchain.toml` L3-5, `clippy.toml` L1-13, `rustfmt.toml` L1, `LICENSE` L1-3, `.cargo/config.toml` L7-17, `.github/workflows/ci.yml` L1-89, `.gitignore` L1, `.gitattributes` L1-4):

- Workspace `Cargo.toml`: `resolver = "3"`, `members = ["crates/*"]`, `[workspace.package]` with `edition = "2024"` and `license = "BSL-1.0"`, and the `[workspace.lints]` rust, rustdoc, and clippy tables verbatim. No `repository` field until the repository has a remote. The crate opts in with `[lints] workspace = true`.
- `rust-toolchain.toml`: channel `1.99` with `rustfmt` and `clippy`. `rustfmt.toml`: `style_edition = "2024"`. `clippy.toml`: verbatim.
- `LICENSE`: verbatim. `.gitignore`: `/target`. `.gitattributes`: `* text=auto eol=lf` only, since there are no C headers.
- `.cargo/config.toml`: the `x86_64-pc-windows-msvc` `rust-lld` linker and `+crt-static`, and `CLIPPY_DISABLE_DOCS_LINKS=1`. Drop the `xtask` alias, since there is no `build-xtask`.
- `.github/workflows/ci.yml`: the same triggers (push to `master` or `main`, and pull requests) and cancellation of superseded runs. `fmt`, `clippy`, and `docs` run on `ubuntu-latest`. `test` runs on `ubuntu-latest`, `windows-latest`, and `macos-latest` with `fail-fast: false`, runs the full suite and the release build of `plugin-bashkit`, and ends with the clean-tree check. Drop the `msrv` job, since the crate is unpublished and the toolchain is pinned.

## Notes

- Commands, from `runvil-plugin-api` `.github/workflows/ci.yml` L1-89 and `AGENTS.md` L38-44, adapted to this crate:
  - Build: `cargo build --locked`. Release library: `cargo build --locked -p plugin-bashkit --release`.
  - Package tests: `cargo test --locked -p plugin-bashkit --all-features`.
  - Full suite: `cargo test --locked --workspace --all-features`.
  - Lint: `CARGO_BUILD_WARNINGS=deny cargo clippy --locked --workspace --all-targets --all-features`.
  - Format: `cargo fmt --all --check`.
  - Docs: `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --all-features`.
- The repository has no commits and no `Cargo.lock`, and `--locked` fails without one, so step 1 runs `cargo generate-lockfile` first and commits `Cargo.lock`.
- `https://github.com/cppalliance/runvil-plugin-api` holds `master` at `63a2403761b925b1f52bcf361c97b4fb04c04d74`, where `runvil-plugin-sdk` is version 0.2.0 (`runvil-plugin-api` `crates/runvil-plugin-sdk/Cargo.toml` L1-21).
- The repository has no remote; nothing is pushed.
- `runvil-plugin-api` and `runvil` are cloned beside this repository, at `../runvil-plugin-api` and `../runvil`. Copy files and read `hello-plugin` from `../runvil-plugin-api`, which is clean at `63a2403761b925b1f52bcf361c97b4fb04c04d74`.

### Open questions

None

</plan-context>

## Steps

<step-1>

1. [done] **Scaffold the repository and a bashkit Plugin that describes itself** (points: 4) - Create the workspace and the configuration files listed under Repository setup, and the `plugin-bashkit` crate at `crates/plugin-bashkit` as a `cdylib` on `runvil-plugin-sdk`. Implement `Bashkit` and its `Plugin` impl, export it with `export_plugin!`, and give the crate its crate-level docs. Add `.github/workflows/ci.yml`. Generate and commit `Cargo.lock`. Tests: a unit test, following `hello-plugin`'s own test of `runvil_plugin_sdk::testing::load`, where loading `Bashkit` through the real entry point succeeds and describes it as `bashkit` at `0.1.0`; the release build of `plugin-bashkit` produces its dynamic library.
   - Score: workspace and `plugin-bashkit` manifests with the SDK dependency 1, `Bashkit` and its `Plugin` impl 1, `export_plugin!` wiring 1, CI workflow 1, configuration files copied from `runvil-plugin-api` 0.

`Cargo.toml`

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
edition = "2024"
license = "BSL-1.0"

# The Plugin SDK at contract 0.2, pinned to the commit
# the Engine's loader speaks.
[workspace.dependencies.runvil-plugin-sdk]
version = "0.2.0"
git = "https://github.com/cppalliance/runvil-plugin-api"
rev = "63a2403761b925b1f52bcf361c97b4fb04c04d74"
# new
```

`crates/plugin-bashkit/Cargo.toml`

```toml
[package]
name = "plugin-bashkit"
version = "0.1.0"
edition.workspace = true
license.workspace = true
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
runvil-plugin-sdk.workspace = true

[dev-dependencies.runvil-plugin-sdk]
workspace = true
features = ["test-support"]

[lints]
workspace = true
# new
```

`crates/plugin-bashkit/src/lib.rs`

```rust
/// The bashkit Plugin's instance, with no state yet. At
/// contract 0.2 it only describes itself; its Pack comes
/// once the contract can register one.
#[derive(Debug)]
pub struct Bashkit;
// new

impl Plugin for Bashkit {
    /// One Pack id segment.
    const NAME: &'static str = "bashkit";
    /// This crate's version.
    const VERSION: &'static str =
        env!("CARGO_PKG_VERSION");
    /// Loads with no state and never refuses.
    fn load() -> Result<Bashkit, Refusal>;
}
// new

/// Exports the C entry point `runvil_plugin`.
runvil_plugin_sdk::export_plugin!(Bashkit);
// new
```

</step-1>
