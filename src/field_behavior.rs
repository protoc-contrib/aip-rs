//! AIP-203 field behavior: operations on a message's fields, chosen by their
//! `google.api.field_behavior`.
//!
//! See [`FieldBehavior`], the [`FieldBehaviorKind`] its operations take, the
//! [`FieldScope`] a check covers, and the [`FieldBehaviorError`] it reports.

use core::fmt;

use crate::FieldPath;

/// A `google.api.field_behavior` value.
///
/// See <https://google.aip.dev/203>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldBehaviorKind {
    /// The field may be omitted.
    Optional,
    /// The field must be set: on a create, every one; on an update, those its
    /// mask names.
    Required,
    /// The field is set by the server, and ignored on input.
    OutputOnly,
    /// The field is accepted on input, and never returned.
    InputOnly,
    /// The field may be set on create, and never changed afterwards.
    Immutable,
    /// A repeated field whose order is not significant.
    UnorderedList,
    /// The field has a non-empty default the server applies.
    NonEmptyDefault,
    /// The field is the resource's `name`.
    Identifier,
}

impl FieldBehaviorKind {
    /// The `google.api.FieldBehavior` value name, e.g. `REQUIRED`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Optional => "OPTIONAL",
            Self::Required => "REQUIRED",
            Self::OutputOnly => "OUTPUT_ONLY",
            Self::InputOnly => "INPUT_ONLY",
            Self::Immutable => "IMMUTABLE",
            Self::UnorderedList => "UNORDERED_LIST",
            Self::NonEmptyDefault => "NON_EMPTY_DEFAULT",
            Self::Identifier => "IDENTIFIER",
        }
    }
}

/// Writes the `google.api.FieldBehavior` value name, e.g. `REQUIRED`.
impl fmt::Display for FieldBehaviorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The kind a `google.api.FieldBehavior` number names, as a decoded field
/// option holds it:
///
/// ```
/// use aip::{FieldBehaviorKind, UnknownFieldBehavior};
///
/// assert_eq!(FieldBehaviorKind::try_from(2), Ok(FieldBehaviorKind::Required));
/// assert_eq!(i32::from(FieldBehaviorKind::Required), 2);
/// // FIELD_BEHAVIOR_UNSPECIFIED, and a value from a newer google.api
/// assert_eq!(FieldBehaviorKind::try_from(0), Err(UnknownFieldBehavior(0)));
/// assert_eq!(FieldBehaviorKind::try_from(9), Err(UnknownFieldBehavior(9)));
/// ```
impl TryFrom<i32> for FieldBehaviorKind {
    type Error = UnknownFieldBehavior;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => Self::Optional,
            2 => Self::Required,
            3 => Self::OutputOnly,
            4 => Self::InputOnly,
            5 => Self::Immutable,
            6 => Self::UnorderedList,
            7 => Self::NonEmptyDefault,
            8 => Self::Identifier,
            _ => return Err(UnknownFieldBehavior(value)),
        })
    }
}

/// The `google.api.FieldBehavior` number of a kind, e.g. `2` for `REQUIRED`.
impl From<FieldBehaviorKind> for i32 {
    fn from(kind: FieldBehaviorKind) -> Self {
        match kind {
            FieldBehaviorKind::Optional => 1,
            FieldBehaviorKind::Required => 2,
            FieldBehaviorKind::OutputOnly => 3,
            FieldBehaviorKind::InputOnly => 4,
            FieldBehaviorKind::Immutable => 5,
            FieldBehaviorKind::UnorderedList => 6,
            FieldBehaviorKind::NonEmptyDefault => 7,
            FieldBehaviorKind::Identifier => 8,
        }
    }
}

/// A `google.api.FieldBehavior` number that names no [`FieldBehaviorKind`]:
/// `FIELD_BEHAVIOR_UNSPECIFIED` (0), or a value from a newer `google.api` than
/// this crate knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownFieldBehavior(pub i32);

impl fmt::Display for UnknownFieldBehavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown google.api.FieldBehavior value {}", self.0)
    }
}

impl core::error::Error for UnknownFieldBehavior {}

impl FieldBehaviorKind {
    /// Every kind, in `google.api.FieldBehavior` order: the order a
    /// [`FieldBehaviorSet`] iterates in.
    const ALL: [Self; 8] = [
        Self::Optional,
        Self::Required,
        Self::OutputOnly,
        Self::InputOnly,
        Self::Immutable,
        Self::UnorderedList,
        Self::NonEmptyDefault,
        Self::Identifier,
    ];

    /// The kind's bit in a [`FieldBehaviorSet`].
    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// A set of [`FieldBehaviorKind`]s: what one field is annotated with, or which
/// kinds an operation should consider.
///
/// ```
/// use aip::{FieldBehaviorKind, FieldBehaviorSet};
///
/// let unwritable = FieldBehaviorKind::OutputOnly | FieldBehaviorKind::Identifier | FieldBehaviorKind::Immutable;
/// assert!(unwritable.contains(FieldBehaviorKind::Identifier));
/// assert!(!unwritable.contains(FieldBehaviorKind::Required));
///
/// let field: FieldBehaviorSet = [FieldBehaviorKind::OutputOnly, FieldBehaviorKind::Immutable].into_iter().collect();
/// assert!(field.intersects(unwritable));
/// assert_eq!(field.iter().collect::<Vec<_>>(), [FieldBehaviorKind::OutputOnly, FieldBehaviorKind::Immutable]);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FieldBehaviorSet(u16);

impl FieldBehaviorSet {
    /// The set with no kinds.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Whether `kind` is in the set.
    #[must_use]
    pub const fn contains(self, kind: FieldBehaviorKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Whether the set shares a kind with `other`.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether the set has no kinds.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The kinds in the set, in `google.api.FieldBehavior` order.
    pub fn iter(self) -> Iter {
        Iter { set: self, next: 0 }
    }
}

impl From<FieldBehaviorKind> for FieldBehaviorSet {
    fn from(kind: FieldBehaviorKind) -> Self {
        Self(kind.bit())
    }
}

impl core::ops::BitOr for FieldBehaviorSet {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr<FieldBehaviorKind> for FieldBehaviorSet {
    type Output = Self;

    fn bitor(self, kind: FieldBehaviorKind) -> Self {
        Self(self.0 | kind.bit())
    }
}

impl core::ops::BitOr for FieldBehaviorKind {
    type Output = FieldBehaviorSet;

    fn bitor(self, other: Self) -> FieldBehaviorSet {
        FieldBehaviorSet(self.bit() | other.bit())
    }
}

impl core::ops::BitOrAssign<FieldBehaviorKind> for FieldBehaviorSet {
    fn bitor_assign(&mut self, kind: FieldBehaviorKind) {
        self.0 |= kind.bit();
    }
}

impl FromIterator<FieldBehaviorKind> for FieldBehaviorSet {
    fn from_iter<I: IntoIterator<Item = FieldBehaviorKind>>(kinds: I) -> Self {
        let mut set = Self::empty();
        for kind in kinds {
            set |= kind;
        }
        set
    }
}

impl IntoIterator for FieldBehaviorSet {
    type Item = FieldBehaviorKind;
    type IntoIter = Iter;

    fn into_iter(self) -> Iter {
        self.iter()
    }
}

/// The kinds in a [`FieldBehaviorSet`], in `google.api.FieldBehavior` order.
#[derive(Debug, Clone)]
pub struct Iter {
    set: FieldBehaviorSet,
    next: usize,
}

impl Iterator for Iter {
    type Item = FieldBehaviorKind;

    fn next(&mut self) -> Option<FieldBehaviorKind> {
        while let Some(&kind) = FieldBehaviorKind::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(kind) {
                return Some(kind);
            }
        }
        None
    }
}

/// Which fields an operation applies to.
///
/// ```
/// use aip::FieldScope;
///
/// let mask = ["item".to_owned()];
/// let scope = FieldScope::Mask(&mask);
/// assert!(scope.contains("item"));
/// assert!(scope.contains("item.title"));
/// assert!(!scope.contains("items"));
/// assert!(FieldScope::All.contains("anything"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldScope<'a> {
    /// Every field, as on a create.
    All,
    /// The fields these field-mask paths name, and everything beneath them, as
    /// on an update. The paths are matched as written: `*` is not expanded, so
    /// pass [`FieldScope::All`] for full replacement.
    Mask(&'a [String]),
}

impl FieldScope<'_> {
    /// Whether the dotted field `path` is in scope: always for
    /// [`All`](Self::All); for [`Mask`](Self::Mask), when a mask path names it
    /// or a field above it. Paths are compared segment by segment, so `item`
    /// contains `item.title` but not `items`, and `item.title` contains only
    /// itself.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        match self {
            Self::All => true,
            Self::Mask(paths) => paths.iter().any(|named| {
                named == path
                    || path
                        .strip_prefix(named.as_str())
                        .is_some_and(|rest| rest.starts_with('.'))
            }),
        }
    }
}

/// Operations on a message's fields, chosen by their `google.api.field_behavior`.
///
/// Implemented by `protoc-gen-rust-aip` for every message it generates. Each
/// method acts on the fields annotated with the [`FieldBehaviorKind`] it is
/// given, and continues into the messages beneath them. The defaults do
/// nothing, so a message with no annotated field has an empty impl, and generic
/// code over `T: FieldBehavior` works for every generated message. The trait is
/// dyn-compatible.
///
/// Every combination of operation and kind is allowed; which ones a service
/// applies, and to which messages, is its own policy. AIP-203 itself defines
/// clearing `OUTPUT_ONLY` fields on input and `INPUT_ONLY` fields on output, and
/// validating `REQUIRED` fields — every one on a create, those the update mask
/// names on an update:
///
/// ```
/// use aip::{FieldBehavior, FieldBehaviorError, FieldBehaviorKind, FieldPath, FieldScope, FieldType};
///
/// #[derive(Default)]
/// struct Book {
///     title: String,
///     create_time: Option<u64>,
/// }
///
/// impl FieldBehavior for Book {
///     fn clear_fields(&mut self, kind: FieldBehaviorKind) {
///         if kind == FieldBehaviorKind::OutputOnly {
///             self.create_time = None;
///         }
///     }
///
///     fn validate_fields(&self, kind: FieldBehaviorKind, scope: FieldScope<'_>) -> Result<(), FieldBehaviorError> {
///         let mut paths = Vec::new();
///         if kind == FieldBehaviorKind::Required && scope.contains("title") && self.title.is_empty() {
///             paths.push(FieldPath::field("title", 1, FieldType::String));
///         }
///         FieldBehaviorError::check(kind, paths)
///     }
/// }
///
/// let mut book = Book { title: String::new(), create_time: Some(7) };
/// book.clear_fields(FieldBehaviorKind::OutputOnly);
/// assert_eq!(book.create_time, None);
///
/// let error = book.validate_fields(FieldBehaviorKind::Required, FieldScope::All).unwrap_err();
/// assert_eq!(error.to_string(), "REQUIRED fields are unset: title");
///
/// // a partial update that does not name `title` may leave it unset
/// let mask = ["description".to_owned()];
/// assert!(book.validate_fields(FieldBehaviorKind::Required, FieldScope::Mask(&mask)).is_ok());
/// ```
///
/// The operations are generated rather than reflective because buffa 0.9 has
/// no reflective path to mutation, and the read-only one is generated alongside
/// so field behavior has one mechanism.
///
/// See <https://google.aip.dev/203>.
pub trait FieldBehavior {
    /// Unsets every field annotated `kind`, at any depth: through singular
    /// message fields, oneof members, and the elements of repeated and map
    /// message fields. A field that is cleared is not descended into.
    ///
    /// Clearing `OUTPUT_ONLY` on an inbound request keeps a client from writing
    /// server-owned values, and clearing `INPUT_ONLY` on an outbound response
    /// keeps them from being returned, as AIP-203 asks:
    ///
    /// ```ignore
    /// request.clear_fields(FieldBehaviorKind::OutputOnly);
    /// response.clear_fields(FieldBehaviorKind::InputOnly);
    /// ```
    ///
    /// Does nothing by default.
    fn clear_fields(&mut self, _kind: FieldBehaviorKind) {}

    /// Fails with the fields annotated `kind` that `scope` contains and that
    /// are unset, in declaration order, depth first; `Ok(())` when there are
    /// none.
    ///
    /// Unset is protobuf presence: a message, `optional` scalar or oneof member
    /// when absent; any other scalar at its zero value, so a `REQUIRED` `bool`
    /// must be `true`; a repeated field or a map when empty.
    ///
    /// The check continues into singular message fields and oneof members that
    /// are set, and reports their fields as dotted paths (`origin.city`) from
    /// `self`; one that is unset is not entered, as a nested message's
    /// annotations apply only when it is set. With a [`FieldScope::Mask`], a
    /// path naming a message field checks everything beneath it, and a path
    /// naming a field beneath it checks just that field — even when the message
    /// is unset, since the update writes the field as empty. Repeated and map
    /// elements are not entered: a mask cannot address an element.
    ///
    /// ```ignore
    /// item.validate_fields(FieldBehaviorKind::Required, FieldScope::All)?;                // create
    /// item.validate_fields(FieldBehaviorKind::Required, FieldScope::Mask(&mask.paths))?;  // update
    /// ```
    ///
    /// Valid by default.
    ///
    /// # Errors
    ///
    /// A [`FieldBehaviorError`] of `kind`, with a path per field that fails.
    fn validate_fields(
        &self,
        _kind: FieldBehaviorKind,
        _scope: FieldScope<'_>,
    ) -> Result<(), FieldBehaviorError> {
        Ok(())
    }
}

/// The fields of one [`FieldBehaviorKind`] that failed a check — the `REQUIRED`
/// fields a create or update leaves unset — returned by
/// [`FieldBehavior::validate_fields`].
///
/// ```
/// use aip::{FieldBehaviorError, FieldBehaviorKind, FieldPath, FieldType};
///
/// let display_name = FieldPath::field("display_name", 2, FieldType::String);
/// let error = FieldBehaviorError::check(FieldBehaviorKind::Required, vec![display_name])
///     .unwrap_err()
///     .prefixed(
///         &FieldPath::index("requests", 1, FieldType::Message, 1)
///             .join(FieldPath::field("item", 1, FieldType::Message)),
///     );
/// assert_eq!(error.to_string(), "REQUIRED fields are unset: requests[1].item.display_name");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldBehaviorError {
    /// The behavior the fields failed.
    pub kind: FieldBehaviorKind,
    /// The fields that failed, from the message checked, in the order it holds
    /// them.
    pub paths: Vec<FieldPath>,
}

impl FieldBehaviorError {
    /// `Ok(())` when `paths` is empty, otherwise the error: how a check returns.
    ///
    /// # Errors
    ///
    /// The error of `kind` holding `paths`, when there is at least one.
    pub fn check(kind: FieldBehaviorKind, paths: Vec<FieldPath>) -> Result<(), Self> {
        if paths.is_empty() {
            Ok(())
        } else {
            Err(Self { kind, paths })
        }
    }

    /// Every path, prefixed with `prefix`: how a caller holding the checked
    /// message inside another reports it, e.g. under `item` or
    /// `requests[1].item`.
    #[must_use]
    pub fn prefixed(mut self, prefix: &FieldPath) -> Self {
        for path in &mut self.paths {
            *path = prefix.clone().join(core::mem::take(path));
        }
        self
    }
}

/// Writes `REQUIRED fields are unset: item.display_name, item.quantity`, the
/// verb depending on the behavior.
impl fmt::Display for FieldBehaviorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let failure = match self.kind {
            FieldBehaviorKind::Required => "are unset",
            FieldBehaviorKind::Immutable => "cannot change",
            _ => "are not valid",
        };
        write!(f, "{} fields {failure}: ", self.kind)?;
        for (i, path) in self.paths.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{path}")?;
        }
        Ok(())
    }
}

impl core::error::Error for FieldBehaviorError {}

/// `FieldBehavior` for the well-known types buffa-types generates: none of
/// their fields is annotated, so the defaults are theirs, and a procedure that
/// takes or returns one -- `google.protobuf.Empty`, say -- is handled like any
/// other.
macro_rules! well_known {
    ($($ty:ident),* $(,)?) => {
        $(impl FieldBehavior for buffa_types::google::protobuf::$ty {})*
    };
}

well_known!(
    Any,
    Api,
    BoolValue,
    BytesValue,
    DoubleValue,
    Duration,
    Empty,
    Enum,
    EnumValue,
    Field,
    FieldMask,
    FloatValue,
    Int32Value,
    Int64Value,
    ListValue,
    Method,
    Mixin,
    Option,
    SourceContext,
    StringValue,
    Struct,
    Timestamp,
    Type,
    UInt32Value,
    UInt64Value,
    Value,
);

#[cfg(test)]
mod tests {
    use super::{
        FieldBehavior, FieldBehaviorError, FieldBehaviorKind, FieldScope, UnknownFieldBehavior,
    };
    use crate::{FieldPath, FieldType};

    fn string(name: &'static str, number: u32) -> FieldPath {
        FieldPath::field(name, number, FieldType::String)
    }

    #[test]
    fn kinds_write_their_proto_names() {
        assert_eq!(FieldBehaviorKind::OutputOnly.to_string(), "OUTPUT_ONLY");
        assert_eq!(
            FieldBehaviorKind::NonEmptyDefault.as_str(),
            "NON_EMPTY_DEFAULT"
        );
        assert_eq!(FieldBehaviorKind::Identifier.as_str(), "IDENTIFIER");
    }

    #[test]
    fn a_kind_round_trips_through_its_number() {
        for value in 1..=8 {
            let kind = FieldBehaviorKind::try_from(value).expect("a known value");
            assert_eq!(i32::from(kind), value);
        }
        assert_eq!(
            FieldBehaviorKind::try_from(3),
            Ok(FieldBehaviorKind::OutputOnly)
        );
        assert_eq!(i32::from(FieldBehaviorKind::Identifier), 8);
    }

    #[test]
    fn unspecified_and_unknown_numbers_are_no_kind() {
        assert_eq!(FieldBehaviorKind::try_from(0), Err(UnknownFieldBehavior(0)));
        assert_eq!(FieldBehaviorKind::try_from(9), Err(UnknownFieldBehavior(9)));
        assert_eq!(
            UnknownFieldBehavior(9).to_string(),
            "unknown google.api.FieldBehavior value 9"
        );
    }

    #[test]
    fn a_set_holds_what_it_is_built_from() {
        use super::FieldBehaviorSet;
        use FieldBehaviorKind::{Identifier, Immutable, InputOnly, OutputOnly, Required};

        let unwritable = OutputOnly | Identifier | Immutable;
        assert!(unwritable.contains(OutputOnly));
        assert!(unwritable.contains(Identifier));
        assert!(unwritable.contains(Immutable));
        assert!(!unwritable.contains(Required));
        assert_eq!(
            unwritable.iter().collect::<Vec<_>>(),
            [OutputOnly, Immutable, Identifier]
        );

        let one = FieldBehaviorSet::from(InputOnly);
        assert!(!one.intersects(unwritable));
        assert!((one | Immutable).intersects(unwritable));
        assert_eq!(one | FieldBehaviorSet::from(Required), Required | InputOnly);

        assert!(FieldBehaviorSet::empty().is_empty());
        assert_eq!(FieldBehaviorSet::default(), FieldBehaviorSet::empty());
        assert!(FieldBehaviorSet::empty().iter().next().is_none());
    }

    #[test]
    fn a_set_collects_and_iterates_in_proto_order() {
        use super::FieldBehaviorSet;
        use FieldBehaviorKind::{Identifier, NonEmptyDefault, Optional, OutputOnly};

        let set: FieldBehaviorSet = [Identifier, Optional, OutputOnly, Optional]
            .into_iter()
            .collect();
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            [Optional, OutputOnly, Identifier]
        );
        assert!(!set.contains(NonEmptyDefault));
        let all: FieldBehaviorSet = FieldBehaviorKind::ALL.into_iter().collect();
        assert_eq!(all.iter().count(), 8);
    }

    #[test]
    fn a_mask_contains_what_it_names_and_what_lies_beneath() {
        let paths = ["item".to_owned(), "origin.city".to_owned()];
        let scope = FieldScope::Mask(&paths);
        assert!(scope.contains("item"));
        assert!(scope.contains("item.title"));
        assert!(scope.contains("origin.city"));
        assert!(!scope.contains("origin"));
        assert!(!scope.contains("origin.street"));
        assert!(!scope.contains("items"));
        assert!(!scope.contains("ite"));
    }

    #[test]
    fn an_empty_mask_contains_nothing_and_all_contains_everything() {
        assert!(!FieldScope::Mask(&[]).contains("item"));
        assert!(FieldScope::All.contains("item.title"));
    }

    #[test]
    fn a_star_is_matched_as_written() {
        let paths = ["*".to_owned()];
        assert!(!FieldScope::Mask(&paths).contains("item"));
    }

    #[test]
    fn check_is_ok_for_no_paths() {
        assert_eq!(
            FieldBehaviorError::check(FieldBehaviorKind::Required, Vec::new()),
            Ok(())
        );
    }

    #[test]
    fn prefixed_prefixes_every_path() {
        let error = FieldBehaviorError::check(
            FieldBehaviorKind::Required,
            vec![
                string("display_name", 2),
                FieldPath::field("origin", 3, FieldType::Message).join(string("city", 1)),
            ],
        )
        .unwrap_err()
        .prefixed(
            &FieldPath::index("requests", 1, FieldType::Message, 1).join(FieldPath::field(
                "item",
                1,
                FieldType::Message,
            )),
        );
        assert_eq!(
            error.to_string(),
            "REQUIRED fields are unset: requests[1].item.display_name, requests[1].item.origin.city"
        );
    }

    #[test]
    fn the_message_reads_the_kind() {
        let immutable = FieldBehaviorError {
            kind: FieldBehaviorKind::Immutable,
            paths: vec![string("sku", 3)],
        };
        assert_eq!(immutable.to_string(), "IMMUTABLE fields cannot change: sku");
        let other = FieldBehaviorError {
            kind: FieldBehaviorKind::Identifier,
            paths: vec![string("name", 1)],
        };
        assert_eq!(other.to_string(), "IDENTIFIER fields are not valid: name");
    }

    #[test]
    fn the_defaults_do_nothing_and_the_trait_is_dyn_compatible() {
        struct Plain;
        impl FieldBehavior for Plain {}

        let mut plain = Plain;
        let behavior: &mut dyn FieldBehavior = &mut plain;
        behavior.clear_fields(FieldBehaviorKind::OutputOnly);
        assert_eq!(
            behavior.validate_fields(FieldBehaviorKind::Required, FieldScope::All),
            Ok(())
        );
    }

    #[test]
    fn a_well_known_type_has_the_defaults() {
        let mut empty = buffa_types::Empty::default();
        empty.clear_fields(FieldBehaviorKind::OutputOnly);
        let mut mask = buffa_types::FieldMask {
            paths: vec!["title".to_owned()],
            ..Default::default()
        };
        mask.clear_fields(FieldBehaviorKind::OutputOnly);
        assert_eq!(mask.paths, ["title"]);
        assert_eq!(
            buffa_types::Timestamp::default()
                .validate_fields(FieldBehaviorKind::Required, FieldScope::All),
            Ok(())
        );
    }
}
