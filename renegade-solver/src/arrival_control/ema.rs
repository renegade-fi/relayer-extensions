//! An implementation of an exponential moving average.

/// The EMA
#[derive(Clone, Copy, Debug)]
pub struct Ema {
    /// The smoothing factor for the EMA.
    alpha: f64,
    /// The current EMA estimate.
    last: Option<f64>,
}

impl Ema {
    /// Construct an EMA with a given smoothing factor α.
    #[inline]
    pub fn with_alpha(alpha: f64) -> Self {
        assert!(alpha > 0.0 && alpha <= 1.0, "alpha must be in (0,1]");
        Self { alpha, last: None }
    }

    /// Construct an EMA from a canonical window length `N`,
    /// using α = 2 / (N + 1).
    ///
    /// This makes the EMA roughly comparable to an N-period SMA,
    /// but smoother and more responsive.
    #[inline]
    pub fn from_window_length(window_length: u32) -> Self {
        assert!(window_length >= 1, "window_length must be >= 1");
        let alpha = 2.0 / (window_length as f64 + 1.0);
        Self::with_alpha(alpha)
    }

    /// Update the EMA with a new observation, returning the new EMA value.
    ///
    /// - On the first call (if not seeded), this initializes EMA = x.
    /// - On subsequent calls, applies EMA_t = α·x + (1−α)·EMA_{t−1}.
    #[inline]
    pub fn update(&mut self, x: f64) -> f64 {
        match self.last {
            None => {
                self.last = Some(x);
                x
            },
            Some(prev) => {
                let a = self.alpha;
                let next = a * x + (1.0 - a) * prev;
                self.last = Some(next);
                next
            },
        }
    }

    /// Returns the current EMA estimate.
    #[inline]
    pub fn last(&self) -> Option<f64> {
        self.last
    }
}
