//! Backend engines for vectorization.
//!
//! Currently supports:
//! - `vtracer` — mature open-source engine with hierarchical color clustering
//! - `hybrid` — vtracer clustering + kurbo curve re-fitting
//! - `logo` — logo-specific pipeline (hard corners, line snapping, shape detection)
//! - `native` — our own pipeline (preprocess → segment → trace → fit → simplify → optimize → output)

pub mod hybrid;
pub mod logo;
pub mod vtracer_backend;
