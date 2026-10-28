//! Request fields: naming one in an error ([`path`], [`error`], AIP-193) and what
//! `google.api.field_behavior` says of one ([`behavior`], AIP-203).
//!
//! Everything is re-exported at the crate root, which is how generated code and
//! a service reach it.

pub mod behavior;
#[cfg(feature = "descriptor")]
pub mod descriptor;
pub mod error;
pub mod path;
