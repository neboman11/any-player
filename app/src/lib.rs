//! Native Slint presentation and typed service ports. Production startup supplies `Services`;
//! `--demo` uses independent, offline in-memory adapters.
pub mod desktop;
pub mod fake;
pub mod images;
mod presenter;
pub mod services;
pub mod startup;
pub mod vm;
// Slint generates implementation details (including an unreachable embedding hook).
// Keep generator-specific lint exceptions out of handwritten application code.
#[allow(clippy::all, clippy::pedantic, clippy::todo)]
mod generated {
    slint::include_modules!();
}
pub use generated::{CardRow, ColumnRow, MainWindow, TrackRow};
pub use presenter::attach;
#[cfg(test)]
mod tests;
