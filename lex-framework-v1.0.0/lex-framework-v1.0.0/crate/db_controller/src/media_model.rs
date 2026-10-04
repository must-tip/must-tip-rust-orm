//! Semantic binary field aliases.
//!
//! Media transport/storage is intentionally kept out of the database model layer. These aliases
//! preserve the existing model API without coupling database code to image/video processing stacks.

use crate::field::BinaryField;

pub type ImageField = BinaryField;
pub type AudioField = BinaryField;
pub type VideoField = BinaryField;
pub type PdfField = BinaryField;
pub type DocumentField = BinaryField;
