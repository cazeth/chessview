//! A PGN viewer for the terminal.
//!
//! [`parser`] turns PGN text into moves, [`game`] replays them on a board, and
//! [`app`] draws the result with ratatui.

// ===========================================================================
// LINTS
//
// Note: these attributes apply to *this crate only*. `main.rs` is a separate
// crate and will not see them — use the `[lints]` table in Cargo.toml if you
// want one setting to cover both.
// ===========================================================================

// Pedantic is a menu, not a mandate: switch it on, then silence what does not
// earn its keep here.
#![warn(clippy::pedantic)]
// A panic inside the alternate screen leaves the user's terminal in raw mode,
// so unwraps in shipped code are worth forbidding outright. Tests may unwrap
// freely — that is what a failing test is for.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
// There is no unsafe code in this crate, and there is no reason for any.
#![deny(unsafe_code)]
// Catches things like `-> Span` where the lifetime should be spelled `Span<'_>`.
#![warn(rust_2018_idioms)]
// -- silenced, with reasons -------------------------------------------------

// A board is 8x8: files and ranks are always 0..8 and offsets are single
// digits, so every `as` cast here is provably in range. Rewriting them with
// `try_from` would add error branches that can never be taken, or `expect`
// calls that could never fire — both hide the invariant behind ceremony.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
// `for rank in 0..8` reads as a coordinate on the board. Clippy's suggested
// nested `enumerate` is harder to follow for a fixed 8x8 grid.
#![allow(clippy::needless_range_loop)]
// Pure getters on small `Copy` types. `#[must_use]` on them catches no real
// bug and would clutter every accessor.
#![allow(clippy::must_use_candidate)]

pub mod action;
pub mod app;
pub mod game;
pub mod parser;

#[cfg(feature = "images")]
mod image_backend;

pub use app::run;
