//! The errors a `List` request's query dimensions fail with.
//!
//! Parsing `filter`, `order_by` and `page_token` is split between this crate
//! and generated code — the ordering and pagination parsers live here, the
//! field declarations a filter is checked against are known only at codegen
//! time — so the error types they share live here, where both can name them.
//!
//! See <https://google.aip.dev/160>, <https://google.aip.dev/132> and
//! <https://google.aip.dev/158>.

use core::fmt;

use crate::{ordering, pagination};

/// Why a `filter` expression was rejected.
///
/// Deliberately not typed over the CEL crate's own error: this crate has no
/// dependencies, which is what lets a caller pick their own CEL version. The
/// syntax message is already rendered by the time it gets here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FilterError {
    /// The expression is not valid CEL.
    Syntax {
        /// What the CEL parser said.
        message: String,
    },
    /// The expression names something the resource does not declare.
    ///
    /// Generated code checks the names an expression references against the
    /// fields of the resource being listed. That is a weaker check than
    /// cel-go's type checker, which the Go implementation uses: an expression
    /// comparing a string field to an integer parses here and fails later, at
    /// the query layer. Names, at least, are caught at the boundary.
    Undeclared {
        /// The name the expression used.
        name: String,
        /// The names it could have used, in declaration order.
        declared: Vec<String>,
    },
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { message } => write!(f, "invalid filter: {message}"),
            Self::Undeclared { name, declared } => {
                write!(
                    f,
                    "invalid filter: {name:?} is not a field of this resource"
                )?;
                if declared.is_empty() {
                    return Ok(());
                }
                f.write_str("; expected one of: ")?;
                for (index, name) in declared.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    f.write_str(name)?;
                }
                Ok(())
            }
        }
    }
}

impl core::error::Error for FilterError {}

/// Why a `List` request's query could not be parsed.
///
/// Every variant maps to `InvalidArgument` at the RPC boundary; they are worth
/// distinguishing in logs, since each says something different about what the
/// client got wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueryError {
    /// The `filter` expression was rejected.
    Filter(FilterError),
    /// The `order_by` string is not AIP-132 ordering syntax.
    OrderBy(ordering::ParseOrderByError),
    /// The `order_by` names a path the resource does not order by.
    NotOrderable(ordering::FieldNotOrderableError),
    /// The `page_token` is malformed, or was issued for another query.
    PageToken(pagination::ParseError),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Filter(error) => error.fmt(f),
            Self::OrderBy(error) => error.fmt(f),
            Self::NotOrderable(error) => error.fmt(f),
            Self::PageToken(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for QueryError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Filter(error) => Some(error),
            Self::OrderBy(error) => Some(error),
            Self::NotOrderable(error) => Some(error),
            Self::PageToken(error) => Some(error),
        }
    }
}

impl From<FilterError> for QueryError {
    fn from(error: FilterError) -> Self {
        Self::Filter(error)
    }
}

impl From<ordering::ParseOrderByError> for QueryError {
    fn from(error: ordering::ParseOrderByError) -> Self {
        Self::OrderBy(error)
    }
}

impl From<ordering::FieldNotOrderableError> for QueryError {
    fn from(error: ordering::FieldNotOrderableError) -> Self {
        Self::NotOrderable(error)
    }
}

impl From<pagination::ParseError> for QueryError {
    fn from(error: pagination::ParseError) -> Self {
        Self::PageToken(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undeclared_name_lists_what_was_available() {
        let error = FilterError::Undeclared {
            name: "titel".to_owned(),
            declared: vec!["title".to_owned(), "author".to_owned()],
        };
        assert_eq!(
            error.to_string(),
            r#"invalid filter: "titel" is not a field of this resource; expected one of: title, author"#
        );
    }

    #[test]
    fn a_resource_declaring_nothing_says_only_that() {
        let error = FilterError::Undeclared {
            name: "title".to_owned(),
            declared: Vec::new(),
        };
        assert_eq!(
            error.to_string(),
            r#"invalid filter: "title" is not a field of this resource"#
        );
    }

    #[test]
    fn a_syntax_error_carries_the_parser_message() {
        let error = FilterError::Syntax {
            message: "unexpected token".to_owned(),
        };
        assert_eq!(error.to_string(), "invalid filter: unexpected token");
    }

    #[test]
    fn a_query_error_displays_as_the_dimension_that_failed() {
        let error = QueryError::from(FilterError::Syntax {
            message: "unexpected token".to_owned(),
        });
        assert_eq!(error.to_string(), "invalid filter: unexpected token");
        assert!(core::error::Error::source(&error).is_some());
    }
}
