//! `google.api.field_behavior` read off a buffa-descriptor [`FieldDescriptor`],
//! for code that works from a descriptor pool rather than from generated
//! types: a test that every writable field has a column, say, or a server layer
//! that inspects a request's schema.
//!
//! Behind the `descriptor` feature. Generated code does not use it: it knows
//! each field's behaviors when it is generated.

use buffa::ExtensionSet;
use buffa::extension::Extension;
use buffa::extension::codecs::{EnumI32, Repeated};
use buffa_descriptor::FieldDescriptor;

use crate::{FieldBehaviorKind, FieldBehaviorSet};

/// `google.api.field_behavior`: extension 1052 of `google.protobuf.FieldOptions`,
/// a repeated `google.api.FieldBehavior`.
const FIELD_BEHAVIOR: Extension<Repeated<EnumI32>> =
    Extension::new(1052, "google.protobuf.FieldOptions");

/// Reads a field's `google.api.field_behavior`.
///
/// ```ignore
/// use aip::{FieldBehaviorKind, FieldDescriptorExt};
///
/// let unwritable = FieldBehaviorKind::OutputOnly | FieldBehaviorKind::Identifier | FieldBehaviorKind::Immutable;
/// let writable: Vec<&str> = pool
///     .message(index)
///     .fields()
///     .iter()
///     .filter(|field| !field.field_behaviors().intersects(unwritable))
///     .map(|field| field.name())
///     .collect();
/// ```
pub trait FieldDescriptorExt {
    /// The behaviors the field is annotated with; empty when it has none.
    /// Values this crate does not know are ignored.
    fn field_behaviors(&self) -> FieldBehaviorSet;
}

impl FieldDescriptorExt for FieldDescriptor {
    fn field_behaviors(&self) -> FieldBehaviorSet {
        self.options()
            .map(|options| options.extension(&FIELD_BEHAVIOR))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| FieldBehaviorKind::try_from(value).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests;
