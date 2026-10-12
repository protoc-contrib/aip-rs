//! AIP-134 update masks and AIP-203 field behavior, as one trait.
//!
//! See [`FieldBehavior`].

/// What `google.api.field_behavior` says a message's fields may do: which ones
/// an update may write, which a create or update must not leave unset, which
/// may never change, and which one side of the wire may not see.
///
/// Implemented by `protoc-gen-rust-aip` for every message with any of these
/// behaviors at some depth — a resource for the update-mask and check items, a
/// request or a message nested in one for the clearing walks. An implementation
/// contains only the items its message needs. Every default is the "nothing to
/// do" answer — no writable paths, nothing missing, nothing changed, nothing to
/// clear — so generic code works for every message the plugin generated an
/// impl for, whether or not that message uses a given behavior: one server
/// layer over `T: FieldBehavior` can clear, imply and check every request it
/// handles.
///
/// The items are generated rather than reflective because buffa 0.9 has no
/// reflective path to mutation, and the read-only ones are generated alongside
/// so field behavior has one mechanism.
///
/// Not dyn-compatible: [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) is an associated
/// constant and [`changed_immutable_paths`](Self::changed_immutable_paths)
/// takes `&Self`. Use generics.
///
/// The methods share names with the AIP concepts rather than being prefixed, so
/// bring the trait into scope where they are called:
///
/// ```
/// use aip::FieldBehavior;
///
/// #[derive(Default)]
/// struct Book {
///     title: String,
///     create_time: Option<u64>,
/// }
///
/// impl FieldBehavior for Book {
///     const MUTABLE_PATHS: &'static [&'static str] = &["title"];
///
///     fn missing_required_paths(&self, mask: Option<&[String]>) -> Vec<&'static str> {
///         let covered = mask.is_none_or(|paths| paths.iter().any(|path| path == "title"));
///         if covered && self.title.is_empty() { vec!["title"] } else { Vec::new() }
///     }
///
///     fn clear_output_only(&mut self) {
///         self.create_time = None;
///     }
/// }
///
/// /// One layer over every request type.
/// fn admit<T: FieldBehavior>(mut resource: T) -> Result<T, Vec<&'static str>> {
///     resource.clear_output_only();
///     let missing = resource.missing_required_paths(None);
///     if missing.is_empty() { Ok(resource) } else { Err(missing) }
/// }
///
/// assert_eq!(admit(Book::default()).err(), Some(vec!["title"]));
/// assert!(Book::default().implied_update_mask().is_empty()); // the default
/// ```
///
/// See <https://google.aip.dev/134> and <https://google.aip.dev/203>.
pub trait FieldBehavior {
    /// The field paths an update may write: every field not annotated
    /// `OUTPUT_ONLY`, `IDENTIFIER` or `IMMUTABLE`, in declaration order.
    ///
    /// This is the *expansion* of the `*` mask, full replacement, which AIP-134
    /// reads as every writable field; an omitted mask is narrower — see
    /// [`implied_update_mask`](Self::implied_update_mask). It is not the check
    /// that a mask names only these: that is `(buf.validate.field).field_mask.in`
    /// on the mask itself, which protovalidate runs along with every other rule
    /// on the request.
    ///
    /// Those two lists say the same thing and are kept in step by a lint
    /// (protoc-gen-aip-lint's `update-mask-writable-fields`).
    ///
    /// Empty by default: a message with no writable fields, or that is not a
    /// resource.
    ///
    /// See <https://google.aip.dev/134#full-replacement>.
    const MUTABLE_PATHS: &'static [&'static str] = &[];

    /// The update mask AIP-134 implies when a client omits one: every field of
    /// [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) this message populates, in
    /// declaration order.
    ///
    /// Populated as protobuf reads presence: a field with explicit presence —
    /// `optional`, a message, a oneof member — when set, even to its zero value;
    /// any other scalar when not its zero value; a repeated field or a map when
    /// not empty. So a zero value without explicit presence is never implied: a
    /// client clears one with an explicit mask, or the schema makes the field
    /// `optional`.
    ///
    /// A populated message field is implied as one path — `carrier`, not
    /// `carrier.name` — so the update writes everything beneath it, nested
    /// `OUTPUT_ONLY` values included. Clear, then imply: call
    /// [`clear_output_only`](Self::clear_output_only) first.
    ///
    /// Reads `self` and returns the paths; store them on the request's
    /// `update_mask` when it is empty. Empty by default.
    ///
    /// See <https://google.aip.dev/134#field-masks>.
    #[must_use]
    fn implied_update_mask(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// The `REQUIRED` fields that are unset, in declaration order.
    ///
    /// AIP-203: a create must set every `REQUIRED` field, so `None` checks them
    /// all. A partial update may omit any of them, but one its mask names must
    /// be set, or the update would clear it, so `Some(paths)` checks only the
    /// fields the mask names — a path naming a message checks everything
    /// beneath it, a nested path just that field. Expand `*` with
    /// [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) first. The parameter is the owned
    /// `FieldMask`'s `paths` as it is, so a request's mask is passed without
    /// conversion:
    ///
    /// ```text
    /// let missing = item.missing_required_paths(None); // create
    /// let missing = item.missing_required_paths(Some(&update_mask.paths)); // update
    /// ```
    ///
    /// Unset is protobuf presence, as
    /// [`implied_update_mask`](Self::implied_update_mask) reads it: a
    /// `REQUIRED` `bool` without explicit presence must be `true`.
    ///
    /// A set singular message is descended into, its own `REQUIRED` fields
    /// reported as dotted paths (`address.city`) — a nested message's
    /// annotations are independent of its parent's. An unset one is not entered
    /// on a create, or when the mask names it whole; its own `REQUIRED` is
    /// reported at its level. But a nested path is checked even when its parent
    /// is unset, reading the parent as empty: the update writes that path, so
    /// it would clear it. Repeated and map fields are never descended into. A
    /// message type that contains itself is descended into once per path.
    ///
    /// This is where `REQUIRED` on a resource is enforced, rather than by
    /// protovalidate: a resource message is shared by its create and update
    /// requests, and a rule on it cannot tell which one it is in. protovalidate
    /// keeps a request's own fields and value rules on the resource's.
    ///
    /// Empty by default: nothing is `REQUIRED`.
    ///
    /// See <https://google.aip.dev/203#required>.
    #[must_use]
    fn missing_required_paths(&self, _mask: Option<&[String]>) -> Vec<&'static str> {
        Vec::new()
    }

    /// The `IMMUTABLE` fields this update sets to a value other than
    /// `existing`'s, in declaration order.
    ///
    /// AIP-203: a service rejects a request that changes an immutable field. An
    /// update that echoes the stored value back, or leaves the field unset,
    /// changes nothing and is not reported. Neither the implied mask nor
    /// [`MUTABLE_PATHS`](Self::MUTABLE_PATHS) ever writes one, so without this
    /// check a changed value would be dropped silently rather than refused. A
    /// repeated field that is also `UNORDERED_LIST` is compared ignoring order,
    /// as a multiset.
    ///
    /// ```text
    /// let changed = update.changed_immutable_paths(&stored);
    /// if !changed.is_empty() { /* InvalidArgument */ }
    /// ```
    ///
    /// Empty by default: nothing is `IMMUTABLE`.
    ///
    /// See <https://google.aip.dev/203#immutable>.
    #[must_use]
    fn changed_immutable_paths(&self, _existing: &Self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Clears every field annotated `OUTPUT_ONLY`, at any depth.
    ///
    /// Call it on an inbound request to drop server-owned values a client
    /// should not be able to set, rather than rejecting the request. A `oneof`
    /// is cleared entirely when the member that is set is output-only.
    ///
    /// Does nothing by default.
    ///
    /// See <https://google.aip.dev/203#output-only>.
    fn clear_output_only(&mut self) {}

    /// Clears every field annotated `INPUT_ONLY`, at any depth.
    ///
    /// Call it on an outbound response, so a value the client may send is never
    /// returned. A `oneof` is cleared entirely when the member that is set is
    /// input-only.
    ///
    /// Does nothing by default.
    ///
    /// See <https://google.aip.dev/203#input-only>.
    fn clear_input_only(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::FieldBehavior;

    /// A message with no field behavior at all: every item is the default.
    #[derive(Default)]
    struct Plain;

    impl FieldBehavior for Plain {}

    #[test]
    fn every_default_is_nothing_to_do() {
        let mut plain = Plain;
        assert!(Plain::MUTABLE_PATHS.is_empty());
        assert!(plain.implied_update_mask().is_empty());
        assert!(plain.missing_required_paths(None).is_empty());
        assert!(
            plain
                .missing_required_paths(Some(&["x".to_owned()]))
                .is_empty()
        );
        assert!(plain.changed_immutable_paths(&Plain).is_empty());
        plain.clear_output_only();
        plain.clear_input_only();
    }

    /// Overrides one item and keeps the rest.
    #[derive(Default)]
    struct Stamped {
        create_time: Option<u64>,
    }

    impl FieldBehavior for Stamped {
        fn clear_output_only(&mut self) {
            self.create_time = None;
        }
    }

    fn admit<T: FieldBehavior>(message: &mut T) -> Vec<&'static str> {
        message.clear_output_only();
        message.missing_required_paths(None)
    }

    #[test]
    fn generic_code_runs_overrides_and_defaults_alike() {
        let mut stamped = Stamped {
            create_time: Some(1),
        };
        assert!(admit(&mut stamped).is_empty());
        assert_eq!(stamped.create_time, None);
        assert!(admit(&mut Plain).is_empty());
    }
}
