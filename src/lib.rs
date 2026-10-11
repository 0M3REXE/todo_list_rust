//! # The library root
//!
//! Everything reusable lives here. `main.rs` runs it; `tests/*.rs` import it.
//! Integration tests are separate crates and cannot import a binary — without
//! this file the tests would have to live inside `src/`.
//!
//! Full walkthrough: `docs/architecture.md`.

pub mod todo;
