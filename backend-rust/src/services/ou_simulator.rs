//! Ornstein-Uhlenbeck micro-tick simulator.
//!
//! Generates realistic intraday price ticks at 1 Hz. The OU process
//! mean-reverts to a base price (from Yahoo Finance) while adding
//! volatility calibrated from the stock's ATR.

use crate::config::{OU_DT, OU_THETA};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rand_distr::Normal;

pub struct OuSimulator {
    price: f64,
    mu: f64,      // mean-reversion target
    theta: f64,   // reversion speed
    sigma: f64,   // volatility
    dt: f64,
    rng: StdRng,
    normal: Normal<f64>,
}

impl OuSimulator {
    pub fn new(base_price: f64, atr: f64) -> Self {
        let sigma = if atr > 0.0 { atr / 60.0 } else { base_price * 0.0003 };
        Self {
            price: base_price,
            mu: base_price,
            theta: OU_THETA,
            sigma,
            dt: OU_DT,
            rng: StdRng::from_entropy(),
            normal: Normal::new(0.0, 1.0).unwrap(),
        }
    }

    /// Update the mean-reversion target (e.g. when Yahoo Finance refreshes).
    pub fn update_mu(&mut self, new_mu: f64, new_atr: f64) {
        self.mu = new_mu;
        if new_atr > 0.0 {
            self.sigma = new_atr / 60.0;
        }
    }

    /// Generate the next tick price (call at 1 Hz).
    pub fn tick(&mut self) -> f64 {
        let dw: f64 = self.rng.sample(self.normal);
        self.price += self.theta * (self.mu - self.price) * self.dt
            + self.sigma * self.dt.sqrt() * dw;
        // Clamp to prevent negative prices
        if self.price < self.mu * 0.95 {
            self.price = self.mu * 0.95;
        } else if self.price > self.mu * 1.05 {
            self.price = self.mu * 1.05;
        }
        self.price
    }

    pub fn current_price(&self) -> f64 {
        self.price
    }
}
