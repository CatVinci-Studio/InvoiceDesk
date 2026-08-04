//! Turning extracted text and structured documents into invoice fields, and
//! checking that the result hangs together.
//!
//! [`text`] is the layout-dependent half: regexes over a flattened text
//! layer. [`validate`] is the layout-independent half: arithmetic and shape
//! checks that hold no matter which extraction layer produced the numbers,
//! which is why they run once at the end of the pipeline rather than inside
//! each extractor.

pub mod text;
pub mod validate;
