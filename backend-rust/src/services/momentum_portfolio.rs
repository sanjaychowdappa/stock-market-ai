//! Low-touch monthly ETF momentum rotation.
//!
//! Purpose: an automated, set-and-forget investing strategy for someone who
//! wants to hold a second job — it rebalances ONCE A MONTH and needs no daily
//! attention. It rotates a diversified ETF universe (the tradeable equivalent
//! of mutual funds — Alpaca can't trade mutual funds, and they price once a
//! day with redemption fees, so ETFs are the right proxy).
//!
//! Signal: ROLLING-AVERAGE (blended) momentum — each ETF is ranked by the
//! average of its 1-, 3-, and 6-month trailing returns. Averaging across
//! horizons is far more robust than a single point-to-point return (which
//! whipsawed badly in the first experiment).
//!
//! Safety: ABSOLUTE momentum — if a would-be holding has negative blended
//! momentum, that slot rotates to a T-bill ETF (cash) instead. This is what
//! sidesteps big drawdowns and makes rotation worthwhile over buy-and-hold.
//!
//! Benchmark: SPY buy-and-hold (the honest yardstick for a diversified
//! tactical strategy). Value is marked to market daily; holdings only change
//! at the monthly rebalance.

use crate::services::alpaca_stream;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

const MOM_START_CASH: f64 = 500.0;
const MOM_TOP_N: usize = 5;
const MOM_LOG: &str = "/app/reports/momentum_portfolio.jsonl";
const MOM_STATE_FILE: &str = "/app/reports/momentum_state.json";
const BENCH_SYMBOL: &str = "SPY";
const CASH_PROXY: &str = "BIL"; // 1-3 month T-bills — the risk-off asset
/// Blended momentum lookbacks in trading days (~1, 3, 6 months).
const MOM_WINDOWS: &[usize] = &[21, 63, 126];

/// Diversified ETF universe — the mutual-fund-equivalent menu: broad equity
/// indices, international, the 11 sector SPDRs, bonds, and alternatives.
const ETF_UNIVERSE: &[&str] = &[
    // Broad US equity
    "SPY", "QQQ", "IWM", "DIA",
    // International
    "EFA", "EEM", "VGK", "EWJ",
    // Sector SPDRs (fund-like exposures)
    "XLK", "XLF", "XLE", "XLV", "XLY", "XLP", "XLI", "XLU", "XLB", "XLRE", "XLC",
    // Bonds (bond funds)
    "TLT", "IEF", "LQD", "HYG", "AGG",
    // Alternatives
    "GLD", "SLV", "DBC",
];

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct MomentumPortfolio {
    cash: f64,
    positions: HashMap<String, f64>, // symbol -> shares
    started_date: String,
    last_rebalance_month: String, // "YYYY-MM"
    rebalances: u32,
    // Benchmark: SPY bought once at the start, held.
    bench_shares: f64,
    bench_start_price: f64,
    // Latest marks
    current_value: f64,
    bench_value: f64,
    top_symbols: Vec<String>,
    // Rolling window of daily edge (portfolio_ret - bench_ret) for a smoothed verdict.
    edge_history: Vec<f64>,
    initialized: bool,
}

impl MomentumPortfolio {
    pub fn to_json(&self) -> Value {
        let ret_pct = if self.initialized {
            (self.current_value - MOM_START_CASH) / MOM_START_CASH * 100.0
        } else { 0.0 };
        let bench_ret_pct = if self.bench_value > 0.0 {
            (self.bench_value - MOM_START_CASH) / MOM_START_CASH * 100.0
        } else { 0.0 };
        let rolling_edge = if self.edge_history.is_empty() { 0.0 }
            else { self.edge_history.iter().sum::<f64>() / self.edge_history.len() as f64 };
        json!({
            "strategy": "monthly ETF momentum rotation (blended 1/3/6-month, absolute-momentum cash switch)",
            "cadence": "monthly rebalance, daily mark-to-market",
            "benchmark": BENCH_SYMBOL,
            "initialized": self.initialized,
            "started_date": self.started_date,
            "last_rebalance_month": self.last_rebalance_month,
            "rebalances": self.rebalances,
            "portfolio_value": (self.current_value * 100.0).round() / 100.0,
            "portfolio_return_pct": (ret_pct * 100.0).round() / 100.0,
            "benchmark_value": (self.bench_value * 100.0).round() / 100.0,
            "benchmark_return_pct": (bench_ret_pct * 100.0).round() / 100.0,
            "edge_vs_benchmark_pct": ((ret_pct - bench_ret_pct) * 100.0).round() / 100.0,
            "rolling_avg_edge_pct": (rolling_edge * 100.0).round() / 100.0,
            "beating_benchmark": self.current_value > self.bench_value,
            "holdings": self.top_symbols,
        })
    }
}

pub type SharedMomentum = Arc<Mutex<MomentumPortfolio>>;

pub fn create_shared() -> SharedMomentum {
    // Restore so the monthly experiment survives the daily restarts. A schema
    // change (from the old daily/stock version) simply fails to parse and
    // starts fresh — which is the intended behaviour for the redesign.
    let mp = std::fs::read_to_string(MOM_STATE_FILE).ok()
        .and_then(|c| serde_json::from_str::<MomentumPortfolio>(&c).ok())
        .unwrap_or(MomentumPortfolio { cash: MOM_START_CASH, ..Default::default() });
    if mp.initialized {
        info!("momentum: restored ETF-rotation state (started {}, last rebalance {})",
            mp.started_date, mp.last_rebalance_month);
    }
    Arc::new(Mutex::new(mp))
}

/// Blended rolling-average momentum: mean of trailing returns over each
/// lookback window. Robust to single-day noise vs a point-to-point return.
fn blended_momentum(closes: &[f64]) -> Option<f64> {
    let n = closes.len();
    let last = *closes.last()?;
    if last <= 0.0 { return None; }
    let mut sum = 0.0;
    let mut cnt = 0;
    for &w in MOM_WINDOWS {
        if n > w {
            let past = closes[n - 1 - w];
            if past > 0.0 {
                sum += (last - past) / past;
                cnt += 1;
            }
        }
    }
    if cnt == 0 { None } else { Some(sum / cnt as f64) }
}

/// Fetch daily closes for a symbol (chronological).
async fn fetch_closes(symbol: &str) -> Vec<f64> {
    match alpaca_stream::fetch_daily_bars(symbol, 140).await {
        Ok(bars) => bars.iter().filter_map(|b| b["close"].as_f64()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Daily tick: mark the portfolio to market; rebalance holdings only when the
/// calendar month has changed (monthly cadence).
pub async fn rebalance(mom: &SharedMomentum) {
    info!("=== ETF MOMENTUM ROTATION (blended {:?}d, monthly) ===", MOM_WINDOWS);

    // 1. Fetch closes for the universe + cash proxy + benchmark.
    let mut prices: HashMap<String, f64> = HashMap::new();
    let mut momentum: HashMap<String, f64> = HashMap::new();
    let mut all_syms: Vec<&str> = ETF_UNIVERSE.to_vec();
    all_syms.push(CASH_PROXY);
    all_syms.push(BENCH_SYMBOL);
    all_syms.sort();
    all_syms.dedup();

    for &sym in &all_syms {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await; // rate limit
        let closes = fetch_closes(sym).await;
        if let Some(&last) = closes.last() {
            if last > 0.0 { prices.insert(sym.to_string(), last); }
        }
        if let Some(m) = blended_momentum(&closes) {
            momentum.insert(sym.to_string(), m);
        }
    }

    let bench_price = prices.get(BENCH_SYMBOL).copied().unwrap_or(0.0);
    if prices.is_empty() {
        warn!("momentum: no ETF prices fetched — skipping");
        return;
    }

    let now = chrono::Utc::now().date_naive();
    let today = now.to_string();
    let this_month = today[..7].to_string(); // "YYYY-MM"

    let mut m = mom.lock();

    // 2. First run: initialise benchmark.
    if !m.initialized {
        m.started_date = today.clone();
        m.cash = MOM_START_CASH;
        if bench_price > 0.0 {
            m.bench_start_price = bench_price;
            m.bench_shares = MOM_START_CASH / bench_price;
        }
        m.initialized = true;
        info!("momentum: initialised ${:.0} ETF rotation on {}", MOM_START_CASH, today);
    }

    // 3. Mark portfolio to market at today's prices.
    let pos_value: f64 = m.positions.iter()
        .map(|(s, sh)| sh * prices.get(s).copied().unwrap_or(0.0))
        .sum();
    let total_value = m.cash + pos_value;
    m.current_value = total_value;
    m.bench_value = if bench_price > 0.0 { m.bench_shares * bench_price } else { m.bench_value };

    // 4. Rebalance holdings only when the month changed (monthly cadence).
    let do_rebalance = m.last_rebalance_month != this_month;
    if do_rebalance {
        // Rank the equity/alt universe (exclude the cash proxy from ranking)
        // by blended momentum, descending.
        let mut ranked: Vec<(&str, f64)> = ETF_UNIVERSE.iter()
            .filter_map(|&s| momentum.get(s).map(|&mo| (s, mo)))
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Top-N with ABSOLUTE momentum: negative-momentum slots go to cash.
        let mut targets: Vec<String> = Vec::new();
        for i in 0..MOM_TOP_N {
            match ranked.get(i) {
                Some(&(sym, mo)) if mo > 0.0 && prices.contains_key(sym) => targets.push(sym.to_string()),
                _ => targets.push(CASH_PROXY.to_string()),
            }
        }

        // Full liquidation → equal-weight buy of the target set.
        m.cash = total_value;
        m.positions.clear();
        let per_slot = total_value / MOM_TOP_N as f64;
        for sym in &targets {
            if let Some(&px) = prices.get(sym) {
                if px > 0.0 {
                    let shares = per_slot / px;
                    *m.positions.entry(sym.clone()).or_insert(0.0) += shares;
                    m.cash -= shares * px;
                }
            }
        }
        m.rebalances += 1;
        m.last_rebalance_month = this_month.clone();
        m.top_symbols = targets.iter().map(|s| {
            let mo = momentum.get(s.as_str()).copied().unwrap_or(0.0);
            format!("{} ({:+.1}%)", s, mo * 100.0)
        }).collect();
        info!("momentum: MONTHLY REBALANCE — new holdings: {:?}", m.top_symbols);
    }

    // 5. Record edge + rolling average.
    let port_ret = (m.current_value - MOM_START_CASH) / MOM_START_CASH * 100.0;
    let bench_ret = if m.bench_value > 0.0 { (m.bench_value - MOM_START_CASH) / MOM_START_CASH * 100.0 } else { 0.0 };
    let edge = port_ret - bench_ret;
    m.edge_history.push(edge);
    if m.edge_history.len() > 20 { let drop_n = m.edge_history.len() - 20; m.edge_history.drain(0..drop_n); }
    let rolling_edge = m.edge_history.iter().sum::<f64>() / m.edge_history.len() as f64;

    info!("momentum: value ${:.2} ({:+.2}%) vs {} ${:.2} ({:+.2}%) | edge {:+.2}% (rolling {:+.2}%) | holdings {:?}",
        m.current_value, port_ret, BENCH_SYMBOL, m.bench_value, bench_ret, edge, rolling_edge, m.top_symbols);

    let log_entry = json!({
        "date": today,
        "rebalanced_today": do_rebalance,
        "portfolio_value": m.current_value,
        "portfolio_return_pct": port_ret,
        "benchmark_value": m.bench_value,
        "benchmark_return_pct": bench_ret,
        "edge_pct": edge,
        "rolling_avg_edge_pct": rolling_edge,
        "beating_benchmark": m.current_value > m.bench_value,
        "holdings": m.top_symbols,
        "rebalances": m.rebalances,
    });
    let state_json = serde_json::to_string(&*m).ok();
    drop(m);
    let log_date = today.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        // AUDIT FIX (2026-07-29): this runs 90s after EVERY restart, and the
        // machine restarts several times a day — so the log accumulated
        // duplicate rows for the same date (Jul 20 had 6), corrupting any
        // history analysis. Rewrite the day's row instead of appending a new
        // one, keeping exactly one authoritative entry per date.
        let existing = tokio::fs::read_to_string(MOM_LOG).await.unwrap_or_default();
        let mut kept: Vec<String> = existing.lines()
            .filter(|l| {
                serde_json::from_str::<Value>(l).ok()
                    .and_then(|v| v["date"].as_str().map(|d| d != log_date))
                    .unwrap_or(false)
            })
            .map(|s| s.to_string())
            .collect();
        kept.push(serde_json::to_string(&log_entry).unwrap_or_default());
        let mut out = kept.join("\n");
        out.push('\n');
        if let Ok(mut f) = tokio::fs::File::create(MOM_LOG).await {
            let _ = f.write_all(out.as_bytes()).await;
        }
        if let Some(sj) = state_json {
            let _ = tokio::fs::write(MOM_STATE_FILE, sj).await;
        }
    });
    info!("=== ETF ROTATION COMPLETE ===");
}
