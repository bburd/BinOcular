//! ```text
//!       __       __ 
//!      (__)_____(__)
//!      |  | |_| |  |
//!      |  |/   \|  |
//!     (____)   (____)
//!
//! ```
//!
//! BinOcular — Know your bytes. Don’t guess them.
//!
//! Precision tooling for peering into binaries, bytecode, and other compiled artifacts.
//! This crate provides the core data structures, error types, and interpretation helpers
//! used throughout the BinOcular ecosystem.
pub mod buffer;
pub mod error;
pub mod interpret;
