//! A path from a message to one of the fields beneath it.
//!
//! See [`FieldPath`].

use core::fmt;

/// A path from a message to a field at any depth beneath it, through singular
/// message fields and list elements: `requests[1].item.display_name`.
///
/// The Rust counterpart of Go's `protopath.Path`, carrying field names only: a
/// caller that needs field numbers or types — to report a protovalidate
/// violation, say — resolves them from its descriptor pool. It is what
/// [`FieldBehavior::missing_required_request_paths`] returns, so a server can
/// report an unset `REQUIRED` field of a batch entry under the entry's index.
///
/// ```
/// use aip::FieldPath;
///
/// let path = FieldPath::index("requests", 1).join(FieldPath::field("item")).join("display_name");
/// assert_eq!(path.to_string(), "requests[1].item.display_name");
/// ```
///
/// [`FieldBehavior::missing_required_request_paths`]: crate::FieldBehavior::missing_required_request_paths
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct FieldPath {
    steps: Vec<Step>,
}

/// One step of a [`FieldPath`]: a field, by name, or an element of the list the
/// step before it names, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// The field with this name, of the message the path has reached.
    Field(&'static str),
    /// The element at this index, of the repeated field the path has reached.
    Index(usize),
}

impl FieldPath {
    /// The path to the field `name`.
    #[must_use]
    pub fn field(name: &'static str) -> Self {
        Self {
            steps: vec![Step::Field(name)],
        }
    }

    /// The path to element `index` of the repeated field `name`: `name[index]`.
    #[must_use]
    pub fn index(name: &'static str, index: usize) -> Self {
        Self {
            steps: vec![Step::Field(name), Step::Index(index)],
        }
    }

    /// This path extended by `path`, which continues from where this one ends:
    /// another [`FieldPath`], or a dotted path such as `address.city`, as
    /// [`FieldBehavior::missing_required_paths`] returns it, read as one field
    /// step per segment.
    ///
    /// [`FieldBehavior::missing_required_paths`]: crate::FieldBehavior::missing_required_paths
    #[must_use]
    pub fn join(mut self, path: impl Into<Self>) -> Self {
        self.steps.extend(path.into().steps);
        self
    }

    /// The steps of this path, from the message it starts at.
    pub fn steps(&self) -> impl ExactSizeIterator<Item = Step> + '_ {
        self.steps.iter().copied()
    }

    /// Whether this path has no steps: the message it starts at.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Reads a dotted path, such as `address.city`, as one field step per segment.
/// An empty string is the empty path.
impl From<&'static str> for FieldPath {
    fn from(path: &'static str) -> Self {
        if path.is_empty() {
            return Self::default();
        }
        Self {
            steps: path.split('.').map(Step::Field).collect(),
        }
    }
}

/// Writes the path as `requests[1].item.display_name`: fields joined by `.`,
/// an index in brackets after the field it is an element of.
impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, step) in self.steps.iter().enumerate() {
            match step {
                Step::Field(name) if i == 0 => f.write_str(name)?,
                Step::Field(name) => write!(f, ".{name}")?,
                Step::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{FieldPath, Step};

    #[test]
    fn displays_fields_and_indices() {
        let path = FieldPath::index("requests", 1)
            .join(FieldPath::field("item"))
            .join("display_name");
        assert_eq!(path.to_string(), "requests[1].item.display_name");
        assert_eq!(FieldPath::field("item").to_string(), "item");
        assert_eq!(FieldPath::default().to_string(), "");
    }

    #[test]
    fn reads_a_dotted_path_as_field_steps() {
        let path = FieldPath::field("item").join("address.city");
        assert_eq!(
            path.steps().collect::<Vec<_>>(),
            [
                Step::Field("item"),
                Step::Field("address"),
                Step::Field("city")
            ]
        );
        assert_eq!(FieldPath::from("address.city").steps().len(), 2);
    }

    #[test]
    fn an_empty_string_is_the_empty_path() {
        assert!(FieldPath::from("").is_empty());
        assert_eq!(FieldPath::field("item").join(""), FieldPath::field("item"));
    }

    #[test]
    fn index_is_the_field_then_the_element() {
        assert_eq!(
            FieldPath::index("requests", 2).steps().collect::<Vec<_>>(),
            [Step::Field("requests"), Step::Index(2)]
        );
    }

    #[test]
    fn equal_paths_hash_alike() {
        use std::collections::HashSet;
        let set: HashSet<_> = [FieldPath::field("a").join("b"), FieldPath::from("a.b")]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 1);
    }
}
