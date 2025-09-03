//! An implementation of a thread-safe exponential moving average.

use std::sync::atomic::Ordering;

use atomic_float::AtomicF64;

/// The EMA
#[derive(Debug)]
pub struct Ema {
    /// The smoothing factor for the EMA.
    alpha: f64,
    /// The current EMA estimate (seeded upfront; non-optional).
    last: AtomicF64,
}

impl Ema {
    /// Construct an EMA with a given smoothing factor α and initial seed value.
    #[inline]
    pub fn with_alpha(alpha: f64, seed: f64) -> Self {
        assert!(alpha > 0.0 && alpha <= 1.0, "alpha must be in (0,1]");
        Self { alpha, last: AtomicF64::new(seed) }
    }

    /// Construct an EMA from a canonical window length `N`,
    /// using α = 2 / (N + 1), and an initial seed value.
    ///
    /// This makes the EMA roughly comparable to an N-period SMA,
    /// but smoother and more responsive.
    #[inline]
    pub fn from_window_length(window_length: u32, seed: f64) -> Self {
        assert!(window_length >= 1, "window_length must be >= 1");
        let alpha = 2.0 / (window_length as f64 + 1.0);
        Self::with_alpha(alpha, seed)
    }

    /// Update the EMA with a new observation, returning the new EMA value.
    /// Applies EMA_t = α·x + (1−α)·EMA_{t−1}.
    #[inline]
    pub fn update(&self, x: f64) -> f64 {
        let prev = self.last.load(Ordering::Relaxed);
        let a = self.alpha;
        let next = a * x + (1.0 - a) * prev;
        self.last.store(next, Ordering::Relaxed);
        next
    }

    /// Returns the current EMA estimate.
    #[inline]
    pub fn last(&self) -> f64 {
        self.last.load(Ordering::Relaxed)
    }
}
