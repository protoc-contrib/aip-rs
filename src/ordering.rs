//! AIP-132 ordering: the `order_by` field of a List request.
//!
//! See <https://google.aip.dev/132#ordering>.

use core::fmt;
use core::str::FromStr;

/// A parsed `order_by` value.
///
/// The grammar is a comma-separated list of field paths, each optionally
/// followed by `asc` or `desc`; an empty string means no ordering. Parse with
/// [`str::parse`]:
///
/// ```
/// use aip::OrderBy;
///
/// let order_by: OrderBy = "author.name, create_time desc".parse()?;
/// assert_eq!(order_by.paths().collect::<Vec<_>>(), ["author.name", "create_time"]);
/// assert_eq!(order_by.to_string(), "author.name, create_time desc");
/// # Ok::<_, aip::ordering::ParseOrderByError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderBy {
    /// The ordering fields, in significance order.
    pub fields: Vec<OrderByField>,
}

/// A single ordering term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderByField {
    /// The field path, dotted for nested fields (`author.name`).
    pub path: String,
    /// Whether the field orders descending.
    pub desc: bool,
}

impl OrderBy {
    /// Returns the ordering field paths, in order.
    ///
    /// The result lines up positionally with a key-set cursor, so it is what
    /// tells you which values to collect for
    /// [`PageToken::next_cursor`](crate::PageToken::next_cursor).
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(|field| field.path.as_str())
    }

    /// Returns whether there is nothing to order by.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Checks that every ordering field is one of `allowed`.
    ///
    /// Use it to enforce the allow-list of orderable fields for a request, so
    /// a client cannot order by a column you did not intend to expose or
    /// index.
    ///
    /// ```
    /// # use aip::OrderBy;
    /// let order_by: OrderBy = "shoe_size".parse()?;
    /// assert!(order_by.validate_for_paths(["title", "create_time"]).is_err());
    /// # Ok::<_, aip::ordering::ParseOrderByError>(())
    /// ```
    pub fn validate_for_paths<I, S>(&self, allowed: I) -> Result<(), FieldNotOrderableError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let allowed: Vec<S> = allowed.into_iter().collect();
        for field in &self.fields {
            if !allowed.iter().any(|path| path.as_ref() == field.path) {
                return Err(FieldNotOrderableError {
                    path: field.path.clone(),
                });
            }
        }
        Ok(())
    }
}

impl OrderByField {
    /// Splits the field path into its segments.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.path.split('.')
    }
}

impl fmt::Display for OrderByField {
    /// Renders the field back into `order_by` syntax.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)?;
        if self.desc {
            f.write_str(" desc")?;
        }
        Ok(())
    }
}

impl fmt::Display for OrderBy {
    /// Renders the ordering back into `order_by` syntax. The result re-parses
    /// to an equal [`OrderBy`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, field) in self.fields.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            field.fmt(f)?;
        }
        Ok(())
    }
}

impl FromStr for OrderBy {
    type Err = ParseOrderByError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = |kind| ParseOrderByError {
            input: s.to_owned(),
            kind,
        };
        if s.trim().is_empty() {
            return Ok(Self::default());
        }
        // Reject the whole input up front, so that a stray character is
        // reported as such rather than as a confusing term or path error.
        if let Some(character) = s
            .chars()
            .find(|c| !c.is_alphabetic() && !c.is_numeric() && !matches!(c, '_' | ' ' | ',' | '.'))
        {
            return Err(error(OrderByErrorKind::InvalidCharacter(character)));
        }
        let terms: Vec<&str> = s.split(',').collect();
        let mut fields: Vec<OrderByField> = Vec::with_capacity(terms.len());
        for term in terms {
            // Only ' ' survives the character check above, so splitting on
            // whitespace is the same split Go's `strings.Fields` performs.
            let parts: Vec<&str> = term.split_whitespace().collect();
            let (path, desc) = match parts[..] {
                // A blank term — a leading, trailing or doubled comma.
                [] => return Err(error(OrderByErrorKind::EmptyFieldPath)),
                [path] => (path, false),
                [path, "asc"] => (path, false),
                [path, "desc"] => (path, true),
                [_, direction] => {
                    return Err(error(OrderByErrorKind::ExpectedAscOrDesc(
                        direction.to_owned(),
                    )));
                }
                _ => return Err(error(OrderByErrorKind::InvalidTerm(term.trim().to_owned()))),
            };
            if path.split('.').any(str::is_empty) {
                return Err(error(OrderByErrorKind::EmptySegment(path.to_owned())));
            }
            // A repeated path is a client bug: the second occurrence can never
            // affect the result, and under key-set pagination it silently
            // desynchronizes the cursor tuple from the sort key.
            if fields.iter().any(|field| field.path == path) {
                return Err(error(OrderByErrorKind::DuplicateField(path.to_owned())));
            }
            fields.push(OrderByField {
                path: path.to_owned(),
                desc,
            });
        }
        Ok(Self { fields })
    }
}

/// Why an `order_by` value could not be parsed.
///
/// Map it to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseOrderByError {
    input: String,
    kind: OrderByErrorKind,
}

impl ParseOrderByError {
    /// Returns the `order_by` value that failed to parse.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Returns what was wrong with it.
    #[must_use]
    pub fn kind(&self) -> &OrderByErrorKind {
        &self.kind
    }
}

/// What was wrong with an `order_by` value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderByErrorKind {
    /// A character that cannot appear in an `order_by` value at all.
    InvalidCharacter(char),
    /// A term with no field path — a leading, trailing or doubled comma.
    EmptyFieldPath,
    /// A field path with an empty segment, such as `author..name`.
    EmptySegment(String),
    /// A second word in a term that is neither `asc` nor `desc`.
    ExpectedAscOrDesc(String),
    /// A term of more than two words.
    InvalidTerm(String),
    /// A field path that appears twice.
    DuplicateField(String),
}

impl fmt::Display for ParseOrderByError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "order_by {:?}: ", self.input)?;
        match &self.kind {
            OrderByErrorKind::InvalidCharacter(character) => {
                write!(f, "invalid character {character:?}")
            }
            OrderByErrorKind::EmptyFieldPath => f.write_str("empty field path"),
            OrderByErrorKind::EmptySegment(path) => {
                write!(f, "field path {path:?}: empty segment")
            }
            OrderByErrorKind::ExpectedAscOrDesc(word) => {
                write!(f, "expected 'asc' or 'desc', got {word:?}")
            }
            OrderByErrorKind::InvalidTerm(term) => write!(f, "invalid term {term:?}"),
            OrderByErrorKind::DuplicateField(path) => write!(f, "duplicate field {path:?}"),
        }
    }
}

impl core::error::Error for ParseOrderByError {}

/// Returned by [`OrderBy::validate_for_paths`] for a field outside the
/// allow-list.
///
/// Map it to `InvalidArgument` at the RPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldNotOrderableError {
    path: String,
}

impl FieldNotOrderableError {
    /// Returns the offending field path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Display for FieldNotOrderableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "order_by: field {:?} is not orderable", self.path)
    }
}

impl core::error::Error for FieldNotOrderableError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<OrderBy, OrderByErrorKind> {
        s.parse::<OrderBy>().map_err(|error| error.kind)
    }

    fn field(path: &str, desc: bool) -> OrderByField {
        OrderByField {
            path: path.to_owned(),
            desc,
        }
    }

    #[test]
    fn parses_the_empty_ordering() {
        for input in ["", "   ", "\t"] {
            assert_eq!(parse(input), Ok(OrderBy::default()), "{input:?}");
        }
    }

    #[test]
    fn parses_directions_and_nesting() {
        assert_eq!(
            parse("title, author.name asc, create_time desc"),
            Ok(OrderBy {
                fields: vec![
                    field("title", false),
                    field("author.name", false),
                    field("create_time", true),
                ],
            })
        );
    }

    #[test]
    fn tolerates_irregular_spacing() {
        assert_eq!(
            parse("  title   desc ,create_time"),
            Ok(OrderBy {
                fields: vec![field("title", true), field("create_time", false)],
            })
        );
    }

    #[test]
    fn round_trips_through_display() {
        for input in ["title", "title desc", "author.name, create_time desc"] {
            let order_by = parse(input).unwrap();
            assert_eq!(parse(&order_by.to_string()), Ok(order_by), "{input:?}");
        }
    }

    #[test]
    fn drops_asc_when_rendering() {
        assert_eq!(parse("title asc").unwrap().to_string(), "title");
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(
            parse("title;"),
            Err(OrderByErrorKind::InvalidCharacter(';'))
        );
        assert_eq!(parse(",title"), Err(OrderByErrorKind::EmptyFieldPath));
        assert_eq!(parse("title,"), Err(OrderByErrorKind::EmptyFieldPath));
        assert_eq!(
            parse("title,,author"),
            Err(OrderByErrorKind::EmptyFieldPath)
        );
        assert_eq!(
            parse("author..name"),
            Err(OrderByErrorKind::EmptySegment("author..name".to_owned()))
        );
        assert_eq!(
            parse("title up"),
            Err(OrderByErrorKind::ExpectedAscOrDesc("up".to_owned()))
        );
        assert_eq!(
            parse("title desc please"),
            Err(OrderByErrorKind::InvalidTerm(
                "title desc please".to_owned()
            ))
        );
    }

    #[test]
    fn rejects_a_repeated_path() {
        assert_eq!(
            parse("title, title desc"),
            Err(OrderByErrorKind::DuplicateField("title".to_owned()))
        );
    }

    #[test]
    fn validates_against_an_allow_list() {
        let order_by = parse("title, create_time desc").unwrap();
        assert!(
            order_by
                .validate_for_paths(["title", "create_time"])
                .is_ok()
        );
        assert_eq!(
            order_by.validate_for_paths(["title"]).unwrap_err().path(),
            "create_time"
        );
        // An empty ordering is orderable by anything, including nothing.
        assert!(OrderBy::default().validate_for_paths::<_, &str>([]).is_ok());
    }

    #[test]
    fn splits_a_path_into_segments() {
        assert_eq!(
            field("book.author.name", false)
                .segments()
                .collect::<Vec<_>>(),
            ["book", "author", "name"]
        );
    }

    #[test]
    fn error_reports_the_offending_input() {
        let error = "title;".parse::<OrderBy>().unwrap_err();
        assert_eq!(error.input(), "title;");
        assert_eq!(
            error.to_string(),
            r#"order_by "title;": invalid character ';'"#
        );
    }
}
