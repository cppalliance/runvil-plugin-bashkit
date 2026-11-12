# runvil-plugin-bashkit

The bashkit Plugin for runvil, built as a dynamic library. It depends on `runvil-plugin-sdk`, from `cppalliance/runvil-plugin-api`, and on no Engine, Harness, or Host crate.

## Definitions

These terms have exactly one meaning each, everywhere in this repository, as in the Engine's repository. Write them capitalized.

- **Engine**: the crates that parse a prompt and step a run. They never see a Plugin.
- **Harness**: the crates that step the Engine and perform its effects. The Harness's loader is the only code that calls this Plugin's entry point in production.
- **Host**: an application that runs prompts through the Harness, such as Workshop. The Host chooses which Plugins load.
- **Pack**: code which installs an optional native prelude, an optional Lua prelude, and zero or more tools.
- **Plugin**: an entry point which installs zero or more Packs. A library Plugin is a dynamic library that exports `runvil_plugin`.

Anything else gets a qualified lowercase name, such as Rust's built-in test harness. Names defined outside this repository are used exactly as defined.

## Principles

- Design for human review. A reviewer reads one crate at a time: its code, its surface, and the items it names from other crates. Keep all three small.
- Do more with less. Before building, check whether an existing facility subsumes the work: reuse one, else make the smallest improvement that enables the feature, else add a new facility, which needs a material benefit beyond tidiness.
- Error and status messages assume a model reads them: concise, factual, and self-contained, naming what is missing or unmet with required versus actual.
- There are no users and no stored data to preserve, so backward compatibility does not matter: change formats, schemas, and APIs freely.

## Engineering

- Prefer types and compiler checks, then behavior tests. Gate on yes/no properties, never on a score threshold that ordinary work edits.
- Behavior changes ship with tests in the same change. Preserve behavior tests during refactors.
- The Plugin crate depends on `runvil-plugin-sdk` and outside libraries alone, and writes no `unsafe`. The SDK owns the C boundary.
- A Plugin shares its process with the Host, so nothing here installs process-global state, such as a panic hook (`clippy.toml` bans `std::panic::set_hook`), and nothing starts a thread that outlives the Plugin's unload. What every run shares lives in the Plugin instance and never in a `static`, so an unload frees it.
- Comments explain a non-obvious constraint, ordering requirement, or workaround. A platform or external-bug workaround cites its upstream issue URL.

## Verification

- Tests, the structural checks among them: `cargo test --locked --workspace --all-features`.
- Lint: `cargo clippy --locked --workspace --all-targets --all-features`, with `CARGO_BUILD_WARNINGS=deny`.
- Format: `cargo fmt --all --check`.
- Docs: `cargo doc --locked --workspace --no-deps --all-features --document-private-items`, with `RUSTDOCFLAGS="-D warnings"`. Almost every item here is private, so the flag is what makes rustdoc check their docs. Clippy does not run rustdoc lints, so never skip this.
- The library Plugin: `cargo build --locked -p plugin-bashkit --release`.

## Structural Rules

- A file holds one concept and has no length limit. Split a file only when it holds a second unrelated concept.
- Size limits sit on functions and types. Clippy holds a function to 100 lines and 7 arguments (`clippy.toml`). `cargo test -p build-checks` holds a type to 25 inherent methods, summed across every `impl` block in its crate, and a struct to 12 named fields. Split an oversized type by responsibility; a new entry in the allowlist in `crates/build-checks/src/type_size.rs` is a design decision with a reason, never a way to make a change pass.
- Production code uses the standard module layout: `foo.rs`, with `foo/bar.rs` for its children. No `#[path]`, and no inline `mod` blocks. A module other than the crate root never has exactly one child file (tests aside): merge it, or make it a peer module `foo_bar.rs` declared beside `foo.rs`.
- Test code goes at the bottom of the file under test in a `#[cfg(test)] mod tests { }` block when practical. Otherwise it goes beside that file, in `foo-tests.rs`, wired with `#[path = "foo-tests.rs"] mod tests;`. A `foo-tests.rs` file is the only kind wired with `#[path]`, so a hyphen in a file name means test code. A helper that the tests of several modules share is a module of its own, declared under `#[cfg(test)]` in the crate root. A top-level `tests/` tree follows Cargo's conventions.
- Pseudo-random test input comes only from a fixed seed that the failure prints.
