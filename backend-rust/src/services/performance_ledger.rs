//! Performance Ledger — persistent cross-day tracking.
//!
//! Saves daily results to disk, calculates net profit after all
//! real-world costs, and provides day-over-day comparison.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use tokio::fs;
use tracing::{info, warn};

const LEDGER_PATH: &str = "/app/reports/performance_ledger.json";

/// Real-world cost model for US equities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostModel {
    /// Average bid-ask spread per share ($)
    pub avg_spread_per_share: f64,
    /// SEC fee per $1000 of sell value
    pub sec_fee_per_1000: f64,
    /// TAF fee per share sold
    pub taf_fee_per_share: f64,
    /// PFOF slippage per share ($)
    pub pfof_slippage_per_share: f64,
    /// Short-term capital gains tax rate
    pub tax_rate: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            avg_spread_per_share: 0.01,
            sec_fee_per_1000: 0.00278,
            taf_fee_per_share: 0.000166,
            pfof_slippage_per_share: 0.002,
            tax_rate: 0.24,
        }
    }
}

/// One day's performance record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayRecord {
    pub date: String,
    /// THIS DAY's gross P&L increment (not the running total).
    pub gross_pnl: f64,
    /// Running total since inception, kept so the next day can compute its
    /// own increment correctly. (Before 2026-07-28 the `gross_pnl` field was
    /// mistakenly storing this cumulative value — see `daily_reliable`.)
    #[serde(default)]
    pub cumulative_pnl: f64,
    /// False for records written before the cumulative-vs-daily bug was fixed;
    /// those rows' `gross_pnl`/`net_pnl` are cumulative and must NOT be summed.
    #[serde(default)]
    pub daily_reliable: bool,
    pub total_trades: u32,
    pub winning_trades: u32,
    pub win_rate: f64,
    pub avg_hold_seconds: u64,
    pub avg_pnl_per_trade: f64,
    pub total_shares_traded: f64,
    pub total_sell_value: f64,
    /// Cost breakdown
    pub spread_cost: f64,
    pub sec_fee: f64,
    pub taf_fee: f64,
    pub pfof_cost: f64,
    pub total_fees: f64,
    pub tax: f64,
    /// Net after all deductions
    pub net_pnl: f64,
    /// Strategy parameters used that day
    pub strategy_params: StrategyParams,
    /// Efficiency score (0-100)
    pub efficiency_score: f64,
    /// Portfolio value at end of day
    pub portfolio_value: f64,
}

/// Snapshot of strategy parameters for the day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyParams {
    pub min_buy_signal: f64,
    pub strong_buy_signal: f64,
    pub sell_signal_threshold: f64,
    pub trailing_stop_pct: f64,
    pub hard_stop_pct: f64,
    pub take_profit_pct: f64,
    pub trade_cooldown_secs: u64,
    pub flat_exit_secs: u64,
    pub max_concurrent_positions: usize,
    pub max_daily_trades: u32,
    pub min_predicted_move_pct: f64,
}

impl StrategyParams {
    pub fn current() -> Self {
        use crate::config::*;
        Self {
            min_buy_signal: MIN_BUY_SIGNAL,
            strong_buy_signal: STRONG_BUY_SIGNAL,
            sell_signal_threshold: SELL_SIGNAL_THRESHOLD,
            // The width the exit ladder actually applies. This used to read
            // TRAILING_STOP_PCT (1.5), which stopped being the operative value
            // when the trail became a fixed 0.75% — every performance record
            // written after that would have documented a stop the system was
            // not using.
            trailing_stop_pct: TRAIL_STOP_FIXED_PCT,
            hard_stop_pct: HARD_STOP_PCT,
            take_profit_pct: TAKE_PROFIT_PCT,
            trade_cooldown_secs: TRADE_COOLDOWN_SECS,
            flat_exit_secs: FLAT_EXIT_SECS,
            max_concurrent_positions: MAX_CONCURRENT_POSITIONS,
            max_daily_trades: MAX_DAILY_TRADES,
            min_predicted_move_pct: MIN_PREDICTED_MOVE_PCT,
        }
    }
}

/// The full ledger — all days.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceLedger {
    pub days: Vec<DayRecord>,
    pub cost_model: CostModel,
}

impl PerformanceLedger {
    pub fn new() -> Self {
        Self {
            days: Vec::new(),
            cost_model: CostModel::default(),
        }
    }

    /// Load ledger from disk, or create new if not found.
    pub async fn load() -> Self {
        match fs::read_to_string(LEDGER_PATH).await {
            Ok(contents) => {
                match serde_json::from_str::<PerformanceLedger>(&contents) {
                    Ok(ledger) => {
                        info!("Loaded performance ledger with {} days", ledger.days.len());
                        ledger
                    }
                    Err(e) => {
                        warn!("Failed to parse ledger: {} — starting fresh", e);
                        Self::new()
                    }
                }
            }
            Err(_) => {
                info!("No existing ledger found — starting fresh");
                Self::new()
            }
        }
    }

    /// Save ledger to disk.
    pub async fn save(&self) {
        let dir = PathBuf::from(LEDGER_PATH).parent().unwrap().to_path_buf();
        let _ = fs::create_dir_all(&dir).await;
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(LEDGER_PATH, json).await {
                    warn!("Failed to save ledger: {}", e);
                }
            }
            Err(e) => warn!("Failed to serialize ledger: {}", e),
        }
    }

    /// Calculate net profit from a day's trading data and record it.
    /// `cumulative_pnl` is the running total since inception; this fn derives
    /// THIS DAY's increment from it (the previous bug recorded the running
    /// total as the daily figure, inflating every "total profit" report).
    pub fn record_day(
        &mut self,
        date: &str,
        cumulative_pnl: f64,
        total_trades: u32,
        winning_trades: u32,
        avg_hold_seconds: u64,
        avg_pnl_per_trade: f64,
        total_shares_traded: f64,
        total_sell_value: f64,
        portfolio_value: f64,
    ) -> DayRecord {
        // THIS DAY's increment = running total minus the last recorded total.
        // Falls back to the raw value on the very first record.
        let prev_cumulative = self.days.last().map(|d| d.cumulative_pnl).unwrap_or(0.0);
        let gross_pnl = cumulative_pnl - prev_cumulative;

        let cm = &self.cost_model;

        // Calculate costs
        let spread_cost = total_shares_traded * cm.avg_spread_per_share;
        let sec_fee = total_sell_value / 1000.0 * cm.sec_fee_per_1000;
        let taf_fee = (total_shares_traded / 2.0) * cm.taf_fee_per_share; // only sells
        let pfof_cost = total_shares_traded * cm.pfof_slippage_per_share;
        let total_fees = spread_cost + sec_fee + taf_fee + pfof_cost;

        // Tax only on positive gains after fees
        let pre_tax_pnl = gross_pnl - total_fees;
        let tax = if pre_tax_pnl > 0.0 { pre_tax_pnl * cm.tax_rate } else { 0.0 };
        let net_pnl = pre_tax_pnl - tax;

        let win_rate = if total_trades > 0 {
            winning_trades as f64 / total_trades as f64 * 100.0
        } else {
            0.0
        };

        // Efficiency score (0-100):
        // Weights: net_pnl (40%), win_rate (20%), avg_hold (15%), trades efficiency (15%), cost ratio (10%)
        let pnl_score = (net_pnl * 10.0).clamp(-50.0, 50.0) + 50.0; // -$5→0, $0→50, $5→100
        let wr_score = win_rate; // 0-100
        let hold_score = ((avg_hold_seconds as f64 / 120.0) * 100.0).clamp(0.0, 100.0); // 120s→100
        let trade_eff = if total_trades > 0 {
            ((winning_trades as f64 * avg_pnl_per_trade.abs()) / (total_fees + 0.001) * 20.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let cost_ratio = if gross_pnl.abs() > 0.001 {
            ((1.0 - total_fees / gross_pnl.abs()) * 100.0).clamp(0.0, 100.0)
        } else {
            50.0
        };

        let efficiency_score = 0.40 * pnl_score
            + 0.20 * wr_score
            + 0.15 * hold_score
            + 0.15 * trade_eff
            + 0.10 * cost_ratio;

        let record = DayRecord {
            date: date.to_string(),
            gross_pnl,
            cumulative_pnl,
            daily_reliable: true,
            total_trades,
            winning_trades,
            win_rate,
            avg_hold_seconds,
            avg_pnl_per_trade,
            total_shares_traded,
            total_sell_value,
            spread_cost,
            sec_fee,
            taf_fee,
            pfof_cost,
            total_fees,
            tax,
            net_pnl,
            strategy_params: StrategyParams::current(),
            efficiency_score,
            portfolio_value,
        };

        // Remove existing entry for same date (re-run)
        self.days.retain(|d| d.date != date);
        self.days.push(record.clone());

        record
    }

    /// Get yesterday's record (or the most recent previous day).
    pub fn yesterday(&self) -> Option<&DayRecord> {
        if self.days.len() >= 2 {
            Some(&self.days[self.days.len() - 2])
        } else {
            None
        }
    }

    /// Get today's record.
    pub fn today(&self) -> Option<&DayRecord> {
        self.days.last()
    }

    /// Compare today vs yesterday — returns improvement analysis.
    pub fn compare_days(&self) -> serde_json::Value {
        let today = match self.today() {
            Some(t) => t,
            None => return json!({"status": "no data"}),
        };

        let yesterday = self.yesterday();

        let mut analysis = json!({
            "today": {
                "date": today.date,
                "gross_pnl": format!("${:.4}", today.gross_pnl),
                "net_pnl": format!("${:.4}", today.net_pnl),
                "total_fees": format!("${:.4}", today.total_fees),
                "tax": format!("${:.4}", today.tax),
                "trades": today.total_trades,
                "win_rate": format!("{:.1}%", today.win_rate),
                "avg_hold": format!("{}s", today.avg_hold_seconds),
                "efficiency": format!("{:.1}", today.efficiency_score),
                "portfolio_value": format!("${:.2}", today.portfolio_value),
            },
            "fee_breakdown": {
                "spread": format!("${:.4}", today.spread_cost),
                "sec_fee": format!("${:.4}", today.sec_fee),
                "taf_fee": format!("${:.4}", today.taf_fee),
                "pfof": format!("${:.4}", today.pfof_cost),
                "total_fees": format!("${:.4}", today.total_fees),
                "tax_24pct": format!("${:.4}", today.tax),
                "gross_to_net": format!("${:.4} → ${:.4}", today.gross_pnl, today.net_pnl),
            },
        });

        if let Some(yday) = yesterday {
            let pnl_change = today.net_pnl - yday.net_pnl;
            let eff_change = today.efficiency_score - yday.efficiency_score;
            let improved = today.net_pnl > yday.net_pnl;

            analysis["yesterday"] = json!({
                "date": yday.date,
                "net_pnl": format!("${:.4}", yday.net_pnl),
                "trades": yday.total_trades,
                "win_rate": format!("{:.1}%", yday.win_rate),
                "efficiency": format!("{:.1}", yday.efficiency_score),
            });
            analysis["comparison"] = json!({
                "pnl_change": format!("{}{:.4}", if pnl_change >= 0.0 { "+" } else { "" }, pnl_change),
                "efficiency_change": format!("{}{:.1}", if eff_change >= 0.0 { "+" } else { "" }, eff_change),
                "improved": improved,
                "verdict": if improved { "IMPROVED" } else { "DEGRADED — auto-tune needed" },
            });
        } else {
            analysis["comparison"] = json!({
                "status": "first day — no comparison available",
                "improved": true,
            });
        }

        // Running totals — ONLY over rows whose daily figures are trustworthy.
        // Pre-fix rows stored the cumulative total in `gross_pnl`, so summing
        // them double-counts (that bug inflated "total profit" ~5x).
        let good: Vec<&DayRecord> = self.days.iter().filter(|d| d.daily_reliable).collect();
        let excluded = self.days.len() - good.len();
        let total_net: f64 = good.iter().map(|d| d.net_pnl).sum();
        let total_gross: f64 = good.iter().map(|d| d.gross_pnl).sum();
        let total_fees: f64 = good.iter().map(|d| d.total_fees).sum();
        let total_tax: f64 = good.iter().map(|d| d.tax).sum();
        let best_day = good.iter().max_by(|a, b| a.net_pnl.partial_cmp(&b.net_pnl).unwrap());
        let worst_day = good.iter().min_by(|a, b| a.net_pnl.partial_cmp(&b.net_pnl).unwrap());
        // The most reliable single figure: the latest running total itself.
        let latest_cumulative = self.days.last().map(|d| d.cumulative_pnl).unwrap_or(0.0);

        analysis["all_time"] = json!({
            "trading_days_counted": good.len(),
            "days_excluded_unreliable": excluded,
            "note": if excluded > 0 {
                format!("{} early rows excluded: they predate the cumulative-vs-daily fix and would double-count.", excluded)
            } else { "all rows reliable".to_string() },
            "latest_cumulative_pnl": format!("${:.4}", latest_cumulative),
            "total_gross_pnl": format!("${:.4}", total_gross),
            "total_fees": format!("${:.4}", total_fees),
            "total_tax": format!("${:.4}", total_tax),
            "total_net_pnl": format!("${:.4}", total_net),
            "best_day": best_day.map(|d| format!("{} (${:.4})", d.date, d.net_pnl)),
            "worst_day": worst_day.map(|d| format!("{} (${:.4})", d.date, d.net_pnl)),
            "avg_daily_net": format!("${:.4}", if good.is_empty() { 0.0 } else { total_net / good.len() as f64 }),
        });

        analysis
    }
}
