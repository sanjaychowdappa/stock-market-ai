//! Kalman Filter for real-time price state estimation.
//!
//! Tracks a 3-state model: [price, velocity, acceleration]
//! - Filters out market microstructure noise from raw ticks
//! - Estimates true price trend and momentum
//! - Predicts future price with confidence intervals
//! - Combines optimally with pattern + Kronos signals
//!
//! The Kalman filter is the mathematically optimal way to estimate
//! hidden state from noisy observations — perfect for tick data.

/// 3-state Kalman filter: price, velocity (trend), acceleration (momentum change).
#[derive(Debug, Clone)]
pub struct PriceKalmanFilter {
    // State vector: [price, velocity, acceleration]
    x: [f64; 3],
    // Covariance matrix (3x3, stored as flat array)
    p: [f64; 9],
    // Process noise (how much we expect state to change per tick)
    q_price: f64,
    q_velocity: f64,
    q_accel: f64,
    // Measurement noise (how noisy are raw tick prices)
    r: f64,
    // Time step
    dt: f64,
    // Tracking
    initialized: bool,
    tick_count: u64,
    // Prediction tracking
    last_prediction: f64,
    prediction_errors: Vec<f64>, // rolling window of prediction errors
}

impl PriceKalmanFilter {
    /// Create a new Kalman filter.
    ///
    /// - `dt`: time step in seconds (1.0 for 1Hz ticks)
    /// - `process_noise`: how volatile the stock is (higher = more responsive, lower = smoother)
    /// - `measurement_noise`: how noisy the raw ticks are (higher = more smoothing)
    pub fn new(dt: f64, process_noise: f64, measurement_noise: f64) -> Self {
        Self {
            x: [0.0; 3],
            p: [
                1.0, 0.0, 0.0,
                0.0, 1.0, 0.0,
                0.0, 0.0, 1.0,
            ],
            q_price: process_noise * 0.1,
            q_velocity: process_noise * 1.0,
            q_accel: process_noise * 5.0,
            r: measurement_noise,
            dt,
            initialized: false,
            tick_count: 0,
            last_prediction: 0.0,
            prediction_errors: Vec::with_capacity(100),
        }
    }

    /// Create with default parameters suitable for stock tick data.
    pub fn default_for_stock(price: f64) -> Self {
        // Scale noise to price level
        let process_noise = price * 0.0001;  // 0.01% of price
        let measurement_noise = price * 0.0005; // 0.05% of price
        let mut kf = Self::new(1.0, process_noise, measurement_noise);
        kf.initialize(price);
        kf
    }

    /// Initialize with first observation.
    pub fn initialize(&mut self, price: f64) {
        self.x = [price, 0.0, 0.0];
        // High initial uncertainty
        self.p = [
            self.r * self.r, 0.0, 0.0,
            0.0, self.r * 10.0, 0.0,
            0.0, 0.0, self.r * 100.0,
        ];
        self.initialized = true;
        self.last_prediction = price;
    }

    /// Predict step — advance state forward in time.
    fn predict(&mut self) {
        let dt = self.dt;
        let dt2 = dt * dt / 2.0;

        // State transition: x_new = F * x
        // price_new = price + velocity*dt + 0.5*accel*dt^2
        // velocity_new = velocity + accel*dt
        // accel_new = accel (constant acceleration model)
        let new_price = self.x[0] + self.x[1] * dt + self.x[2] * dt2;
        let new_vel = self.x[1] + self.x[2] * dt;
        let new_accel = self.x[2] * 0.95; // Slight decay — acceleration doesn't persist forever

        // Covariance prediction: P_new = F * P * F' + Q
        // For 3x3 this is manageable inline
        let f00 = 1.0; let f01 = dt; let f02 = dt2;
        let f10 = 0.0; let f11 = 1.0; let f12 = dt;
        let f20 = 0.0; let f21 = 0.0; let f22 = 0.95;

        let p = &self.p;

        // FP = F * P
        let fp00 = f00*p[0] + f01*p[3] + f02*p[6];
        let fp01 = f00*p[1] + f01*p[4] + f02*p[7];
        let fp02 = f00*p[2] + f01*p[5] + f02*p[8];
        let fp10 = f10*p[0] + f11*p[3] + f12*p[6];
        let fp11 = f10*p[1] + f11*p[4] + f12*p[7];
        let fp12 = f10*p[2] + f11*p[5] + f12*p[8];
        let fp20 = f20*p[0] + f21*p[3] + f22*p[6];
        let fp21 = f20*p[1] + f21*p[4] + f22*p[7];
        let fp22 = f20*p[2] + f21*p[5] + f22*p[8];

        // P_new = FP * F' + Q
        self.p[0] = fp00*f00 + fp01*f01 + fp02*f02 + self.q_price;
        self.p[1] = fp00*f10 + fp01*f11 + fp02*f12;
        self.p[2] = fp00*f20 + fp01*f21 + fp02*f22;
        self.p[3] = fp10*f00 + fp11*f01 + fp12*f02;
        self.p[4] = fp10*f10 + fp11*f11 + fp12*f12 + self.q_velocity;
        self.p[5] = fp10*f20 + fp11*f21 + fp12*f22;
        self.p[6] = fp20*f00 + fp21*f01 + fp22*f02;
        self.p[7] = fp20*f10 + fp21*f11 + fp22*f12;
        self.p[8] = fp20*f20 + fp21*f21 + fp22*f22 + self.q_accel;

        self.x = [new_price, new_vel, new_accel];
    }

    /// Update step — incorporate new price observation.
    fn update(&mut self, observed_price: f64) {
        // Innovation (measurement residual)
        let y = observed_price - self.x[0]; // H = [1, 0, 0]

        // Innovation covariance: S = H*P*H' + R = P[0][0] + R
        let s = self.p[0] + self.r * self.r;

        if s.abs() < 1e-12 { return; }

        // Kalman gain: K = P*H' / S = [P[0]/S, P[3]/S, P[6]/S]
        let k0 = self.p[0] / s;
        let k1 = self.p[3] / s;
        let k2 = self.p[6] / s;

        // State update: x = x + K*y
        self.x[0] += k0 * y;
        self.x[1] += k1 * y;
        self.x[2] += k2 * y;

        // Covariance update: P = (I - K*H) * P
        let i_kh_00 = 1.0 - k0;
        let i_kh_10 = -k1;
        let i_kh_20 = -k2;

        let p = self.p;
        self.p[0] = i_kh_00 * p[0];
        self.p[1] = i_kh_00 * p[1];
        self.p[2] = i_kh_00 * p[2];
        self.p[3] = i_kh_10 * p[0] + p[3];
        self.p[4] = i_kh_10 * p[1] + p[4];
        self.p[5] = i_kh_10 * p[2] + p[5];
        self.p[6] = i_kh_20 * p[0] + p[6];
        self.p[7] = i_kh_20 * p[1] + p[7];
        self.p[8] = i_kh_20 * p[2] + p[8];
    }

    /// Process a new tick price. Returns the filtered state.
    pub fn process_tick(&mut self, observed_price: f64) -> KalmanState {
        if !self.initialized {
            self.initialize(observed_price);
            self.tick_count = 1;
            return self.state();
        }

        // Track prediction accuracy
        let pred_error = observed_price - self.last_prediction;
        self.prediction_errors.push(pred_error);
        if self.prediction_errors.len() > 100 {
            self.prediction_errors.remove(0);
        }

        // Predict → Update cycle
        self.predict();
        self.update(observed_price);

        self.tick_count += 1;

        // Store prediction for next tick
        self.last_prediction = self.x[0] + self.x[1] * self.dt;

        self.state()
    }

    /// Get current estimated state.
    pub fn state(&self) -> KalmanState {
        let price_uncertainty = self.p[0].sqrt();
        let velocity_uncertainty = self.p[4].sqrt();

        // Prediction accuracy (RMSE of recent predictions)
        let pred_accuracy = if self.prediction_errors.len() > 10 {
            let n = self.prediction_errors.len() as f64;
            let mse: f64 = self.prediction_errors.iter().map(|e| e * e).sum::<f64>() / n;
            mse.sqrt()
        } else {
            f64::MAX
        };

        KalmanState {
            // Filtered estimates
            price: self.x[0],
            velocity: self.x[1],       // $/second — positive = rising
            acceleration: self.x[2],    // $/s² — positive = accelerating up

            // Derived signals
            momentum: self.x[1] * 100.0, // Scaled for signal strength
            momentum_change: self.x[2] * 1000.0, // Is momentum increasing or fading?
            trend_strength: self.x[1].abs() / (price_uncertainty + 1e-10), // Signal-to-noise

            // Predictions
            price_5s: self.x[0] + self.x[1] * 5.0 + self.x[2] * 12.5,
            price_30s: self.x[0] + self.x[1] * 30.0 + self.x[2] * 450.0,
            price_60s: self.x[0] + self.x[1] * 60.0 + self.x[2] * 1800.0,

            // Confidence
            price_uncertainty,
            velocity_uncertainty,
            prediction_rmse: pred_accuracy,

            // Direction — require 2x uncertainty to declare direction (was 0.5x)
            // At tick resolution, noise easily exceeds 0.5x. 2x means genuine sustained move.
            direction: if self.x[1] > velocity_uncertainty * 2.0 {
                "bullish"
            } else if self.x[1] < -velocity_uncertainty * 2.0 {
                "bearish"
            } else {
                "neutral"
            },

            tick_count: self.tick_count,
        }
    }

    /// Predict price N seconds ahead.
    pub fn predict_price(&self, seconds: f64) -> (f64, f64) {
        let predicted = self.x[0] + self.x[1] * seconds + 0.5 * self.x[2] * seconds * seconds;
        let uncertainty = (self.p[0] + self.p[4] * seconds * seconds).sqrt();
        (predicted, uncertainty)
    }

    /// Adaptive noise: adjust measurement noise based on recent volatility.
    pub fn adapt_noise(&mut self, recent_volatility: f64) {
        // If volatility is high, trust measurements less (more smoothing)
        self.r = recent_volatility.max(self.x[0].abs() * 0.0001);
    }
}

/// Output of the Kalman filter — the estimated state.
#[derive(Debug, Clone)]
pub struct KalmanState {
    /// Filtered (smoothed) price estimate
    pub price: f64,
    /// Estimated price velocity ($/second) — the true trend
    pub velocity: f64,
    /// Estimated acceleration — is momentum building or fading?
    pub acceleration: f64,
    /// Momentum signal (scaled velocity for trading decisions)
    pub momentum: f64,
    /// Momentum change signal (is momentum accelerating?)
    pub momentum_change: f64,
    /// Trend strength (signal-to-noise ratio of velocity)
    pub trend_strength: f64,
    /// Predicted price in 5 seconds
    pub price_5s: f64,
    /// Predicted price in 30 seconds
    pub price_30s: f64,
    /// Predicted price in 60 seconds
    pub price_60s: f64,
    /// Price estimate uncertainty (std dev)
    pub price_uncertainty: f64,
    /// Velocity estimate uncertainty
    pub velocity_uncertainty: f64,
    /// Prediction RMSE (lower = more accurate recent predictions)
    pub prediction_rmse: f64,
    /// Direction: "bullish", "bearish", "neutral"
    pub direction: &'static str,
    /// Number of ticks processed
    pub tick_count: u64,
}

impl KalmanState {
    /// Confidence score 0-1 (how reliable is the current estimate)
    pub fn confidence(&self) -> f64 {
        if self.tick_count < 10 { return 0.0; }
        let noise_ratio = self.price_uncertainty / (self.price.abs() + 1e-10);
        (1.0 - noise_ratio * 100.0).clamp(0.0, 1.0)
    }

    /// Is there a strong, confirmed trend?
    pub fn has_strong_trend(&self) -> bool {
        self.trend_strength > 1.5 && self.tick_count > 20
    }

    /// Is momentum building (accelerating in trend direction)?
    pub fn momentum_building(&self) -> bool {
        (self.velocity > 0.0 && self.acceleration > 0.0)
            || (self.velocity < 0.0 && self.acceleration < 0.0)
    }

    /// Is momentum fading (decelerating)?
    pub fn momentum_fading(&self) -> bool {
        (self.velocity > 0.0 && self.acceleration < 0.0)
            || (self.velocity < 0.0 && self.acceleration > 0.0)
    }
}
