//! `build-checks` - this repository's structural checks. There is one
//! today: the size limit on types. A check returns a list of violations a
//! person can read, and a test asserts the list is empty, so
//! `cargo test -p build-checks` enforces a rule in `AGENTS.md` that the
//! compiler and clippy cannot.
//!
//! ## Invariants
//!
//! - A check reads source files and never builds or runs the code it
//!   checks.
//! - A file a check cannot read or parse is a violation, never a file to
//!   skip.

mod type_size;

pub use type_size::type_size_violations;
