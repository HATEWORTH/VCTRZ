//! Parallelism and platform abstractions.
//!
//! When the `parallel` feature is enabled, uses rayon for parallel iteration.
//! Without it (e.g. WASM), falls back to standard sequential iterators.
//!
//! Also provides `instant_now()` that works on all platforms including WASM.

// Re-export rayon prelude when parallel is enabled, so callers can just
// `use crate::par::iter_prelude::*;`
pub mod iter_prelude {
    #[cfg(feature = "parallel")]
    pub use rayon::prelude::*;
}

/// Macro that calls `.par_iter()` with rayon or `.iter()` without.
macro_rules! maybe_par_iter {
    ($slice:expr) => {{
        #[cfg(feature = "parallel")]
        { $slice.par_iter() }
        #[cfg(not(feature = "parallel"))]
        { $slice.iter() }
    }};
}

/// Macro that calls `.into_par_iter()` with rayon or `.into_iter()` without.
macro_rules! maybe_into_par_iter {
    ($vec:expr) => {{
        #[cfg(feature = "parallel")]
        { $vec.into_par_iter() }
        #[cfg(not(feature = "parallel"))]
        { $vec.into_iter() }
    }};
}

pub(crate) use maybe_par_iter;
pub(crate) use maybe_into_par_iter;

// ── Instant replacement for WASM ──

/// On native platforms, returns `std::time::Instant::now()`.
/// On WASM (where `Instant` panics), returns a dummy value.
#[cfg(not(target_arch = "wasm32"))]
pub fn instant_now() -> std::time::Instant {
    std::time::Instant::now()
}

#[cfg(target_arch = "wasm32")]
pub fn instant_now() -> WasmInstant {
    WasmInstant
}

/// Dummy Instant for WASM — all duration calculations return zero.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct WasmInstant;

#[cfg(target_arch = "wasm32")]
impl WasmInstant {
    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

#[cfg(target_arch = "wasm32")]
impl std::ops::Add<std::time::Duration> for WasmInstant {
    type Output = Self;
    fn add(self, _: std::time::Duration) -> Self { self }
}

#[cfg(target_arch = "wasm32")]
impl PartialEq for WasmInstant {
    fn eq(&self, _: &Self) -> bool { true }
}

#[cfg(target_arch = "wasm32")]
impl PartialOrd for WasmInstant {
    fn partial_cmp(&self, _: &Self) -> Option<std::cmp::Ordering> {
        Some(std::cmp::Ordering::Less) // deadline never exceeded
    }
}

#[cfg(target_arch = "wasm32")]
impl std::ops::Sub for WasmInstant {
    type Output = std::time::Duration;
    fn sub(self, _: Self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}
