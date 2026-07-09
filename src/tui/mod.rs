//! Terminal UI.
//!
//! Screens and widgets live here; for now just the color [`theme`]
//! palette, which every widget will draw from once the screens land
//! (E5). Kept free of terminal I/O so the palette is unit-testable.

pub mod theme;

pub use theme::{ColorCap, Theme};
