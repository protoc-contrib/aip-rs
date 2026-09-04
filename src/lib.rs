//! Runtime primitives for the Google API Improvement Proposals.
//!
//! This crate holds the parts of AIP support that are pure data manipulation:
//! encoding a page token, parsing an `order_by`, matching a resource name
//! against its pattern. Anything that needs to walk a protobuf message is
//! generated instead — see the scope note in the README.
//!
//! See <https://google.aip.dev>.

// Planned modules, in the order they are worth building:
//
//   page_token  AIP-158. The wire format is specified in docs/page-token.md
//               and is the piece most worth getting right first, because a
//               token issued by the Go implementation must decode here.
//   ordering    AIP-132. `order_by` string -> ordered field paths.
//   resource    AIP-122. Compile a pattern once; scan and format against it.
