//! What a Connect server does with a request before its handler: the resources
//! it writes ([`writes`], AIP-133, AIP-134, AIP-233, AIP-234), the field
//! behavior policy ([`behavior`]), and the hook that readies it ([`prepare`]).
//!
//! Everything is re-exported at the crate root, which is how generated code and
//! a service reach it.

pub mod behavior;
pub mod prepare;
pub mod writes;
