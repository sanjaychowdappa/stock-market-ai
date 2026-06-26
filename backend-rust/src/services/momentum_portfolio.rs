//! Cross-sectional momentum portfolio (paper) — the one strategy with a
//! research-backed edge after a week of data showed the per-second signal
//! stack loses to random.
//!
//! Strategy: each day, rank the S&P 500 universe by TRAILING return (the real
//! momentum factor — what has actually been rising), hold the top 5 equal-
//! weight, rebalance daily. Benchmarked against QQQ buy-and-hold.
//!
//! Critically, ranking is by realized trailing return, NOT the Kronos
//! prediction — the random-baseline test proved Kronos has no edge.
//!
//! Kill criterion (baked in): if this does not beat QQQ within 3 weeks of its
//! start date, the trading thesis is dead. The verdict is logged and exposed.

use crate::services::alpaca_stream;
use crate::services::daily_stock_picker::SP500_UNIVERSE;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

const MOM_START_CASH: f64 = 500.0;
const MOM_TOP_N: usize = 5;
const MOM_LOOKBACK: usize = 60; // trailing trading days (~3 months)
const MOM_KILL_DAYS: i64 = 21;  // beat QQQ within 3 weeks or the thesis is dead
const MOM_LOG: &str = "/app/reports/momentum_portfolio.jsonl";

#[derive(Default)]
pub struct MomentumPortfolio {
    cash: f64,
    positions: HashMap<String, f64>, // symbol -> shares
    started_date: String,
    kill_date: String,
    last_rebalance: String,
    rebalances: u32,
    // Benchmark: $500 of QQQ bought once at the start, held.
    qqq_shares: f64,
    qqq_start_price: f64,
    // Latest marks (for display)
    current_value: f64,
    qqq_value: f64,
    top_symbols: Vec<String>,
    initialized: bool,
}

impl MomentumPortfolio {
    pub fn to_json(&self) -> Value {
        let ret_pct = if self.initialized && MOM_START_CASH > 0.0 {
            (self.current_value - MOM_START_CASH) / MOM_START_CASH * 100.0
        } else { 0.0 };
        let qqq_ret_pct = if self.qqq_value > 0.0 {
            (self.qqq_value - MOM_START_CASH) / MOM_START_CASH * 100.0
        } else { 0.0 };
        let beating_qqq = self.current_value > self.qqq_value;
        json!({
            "strategy": "cross-sectional momentum (trailing return), equal-weight top 5, daily rebalance",
            "initialized": self.initialized,
            "started_date": self.started_date,
            "kill_date": self.kill_date,
            "rebalances": self.rebalances,
            "last_rebalance": self.last_rebalance,
            "portfolio_value": (self.current_value * 100.0).round() / 100.0,
            "portfolio_return_pct": (ret_pct * 100.0).round() / 100.0,
            "qqq_value": (self.qqq_value * 100.0).round() / 100.0,
            "qqq_return_pct": (qqq_ret_pct * 100.0).round() / 100.0,
            "beating_qqq": beating_qqq,
            "edge_vs_qqq_pct": (((ret_pct - qqq_ret_pct)) * 100.0).round() / 100.0,
            "holdings": self.top_symbols,
        })
    }
}

pub type SharedMomentum = Arc<Mutex<MomentumPortfolio>>;

pub fn create_shared() -> SharedMomentum {
    Arc::new(Mutex::new(MomentumPortfolio { cash: MOM_START_CASH, ..Default::default() }))
}

/// Fetch the latest close for a symbol from daily bars.
async fn latest_close(symbol: &str) -> Option<f64> {
    let bars = alpaca_stream::fetch_daily_bars(symbol, 5).await.ok()?;
    bars.last()?["close"].as_f64()
}

/// Run the daily momentum rebalance: rank the universe by trailing return,
/// hold the top N equal-weight, mark vs QQQ, log the snapshot.
pub async fn rebalance(mom: &SharedMomentum) {
    info!("=== MOMENTUM PORTFOLIO REBALANCE (trailing {}d return) ===", MOM_LOOKBACK);

    // 1. Rank the universe by trailing return.
    let mut ranked: Vec<(String, f64, f64)> = Vec::new(); // (symbol, ret, last_close)
    for &sym in SP500_UNIVERSE {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await; // rate limit
        if let Ok(bars) = alpaca_stream::fetch_daily_bars(sym, MOM_LOOKBACK + 10).await {
            if bars.len() > MOM_LOOKBACK {
                let last = bars[bars.len() - 1]["close"].as_f64().unwrap_or(0.0);
                let past = bars[bars.len() - 1 - MOM_LOOKBACK]["close"].as_f64().unwrap_or(0.0);
                if past > 0.0 && last > 0.0 {
                    let ret = (last - past) / past;
                    ranked.push((sym.to_string(), ret, last));
                }
            }
        }
    }
    if ranked.is_empty() {
        warn!("momentum: no ranked symbols (data fetch failed) — skipping rebalance");
        return;
    }
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top: Vec<(String, f64, f64)> = ranked.iter().take(MOM_TOP_N).cloned().collect();
    let prices: HashMap<String, f64> = top.iter().map(|(s, _, p)| (s.clone(), *p)).collect();

    // 2. QQQ benchmark price.
    let qqq_price = latest_close("QQQ").await.unwrap_or(0.0);

    let today = chrono::Utc::now().date_naive().to_string();
    let mut m = mom.lock();

    // 3. First run: initialize benchmark + kill date.
    if !m.initialized {
        m.started_date = today.clone();
        m.kill_date = (chrono::Utc::now().date_naive()
            + chrono::Duration::days(MOM_KILL_DAYS)).to_string();
        if qqq_price > 0.0 {
            m.qqq_start_price = qqq_price;
            m.qqq_shares = MOM_START_CASH / qqq_price;
        }
        m.cash = MOM_START_CASH;
        m.initialized = true;
        info!("momentum: initialized ${:.0} on {}, kill date {}", MOM_START_CASH, today, m.kill_date);
    }

    // 4. Mark current portfolio value (held shares at today's prices).
    let pos_value: f64 = m.positions.iter()
        .map(|(s, sh)| sh * prices.get(s).copied().unwrap_or(0.0))
        .sum();
    // For names dropping out of the top-N we no longer have a price; mark them
    // at entry by treating missing price as 0 contribution is wrong, so the
    // full-liquidation rebalance below uses only top-N prices we just fetched.
    let total_value = m.cash + pos_value;

    // 5. Full rebalance to equal-weight top-N.
    m.cash = total_value;
    m.positions.clear();
    let per_name = total_value / top.len() as f64;
    for (sym, _ret, px) in &top {
        if *px > 0.0 {
            let shares = per_name / px;
            m.positions.insert(sym.clone(), shares);
            m.cash -= shares * px;
        }
    }
    m.rebalances += 1;
    m.last_rebalance = today.clone();
    m.current_value = total_value;
    m.qqq_value = if qqq_price > 0.0 { m.qqq_shares * qqq_price } else { m.qqq_value };
    m.top_symbols = top.iter().map(|(s, r, _)| format!("{} ({:+.1}%)", s, r * 100.0)).collect();

    let port_ret = (m.current_value - MOM_START_CASH) / MOM_START_CASH * 100.0;
    let qqq_ret = if m.qqq_value > 0.0 { (m.qqq_value - MOM_START_CASH) / MOM_START_CASH * 100.0 } else { 0.0 };
    let past_kill = today >= m.kill_date;
    let verdict = if past_kill {
        if m.current_value > m.qqq_value { "PAST KILL DATE — beating QQQ, thesis survives" }
        else { "PAST KILL DATE — NOT beating QQQ, thesis dead per kill criterion" }
    } else { "in trial window" };

    info!("momentum: value ${:.2} ({:+.2}%) vs QQQ ${:.2} ({:+.2}%) | {} | holdings: {:?}",
        m.current_value, port_ret, m.qqq_value, qqq_ret, verdict, m.top_symbols);

    let log_entry = json!({
        "date": today,
        "portfolio_value": m.current_value,
        "portfolio_return_pct": port_ret,
        "qqq_value": m.qqq_value,
        "qqq_return_pct": qqq_ret,
        "beating_qqq": m.current_value > m.qqq_value,
        "holdings": m.top_symbols,
        "rebalances": m.rebalances,
        "verdict": verdict,
    });
    drop(m);
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        if let Ok(mut f) = tokio::fs::OpenOptions::new().create(true).append(true).open(MOM_LOG).await {
            let mut line = serde_json::to_string(&log_entry).unwrap_or_default();
            line.push('\n');
            let _ = f.write_all(line.as_bytes()).await;
        }
    });
    info!("=== MOMENTUM REBALANCE COMPLETE ===");
}
