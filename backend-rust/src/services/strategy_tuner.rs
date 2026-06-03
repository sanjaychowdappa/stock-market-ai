//! Strategy Auto-Tuner — adjusts parameters based on daily performance.
//!
//! At EOD, analyzes what went wrong and adjusts parameters for tomorrow.
//! Saves tuned parameters to disk so the next startup uses them.

use crate::services::performance_ledger::{DayRecord, PerformanceLedger, StrategyParams};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

const TUNED_PARAMS_PATH: &str = "/app/reports/tuned_params.json";

/// Tuned parameters that override config defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunedParams {
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
    pub tuned_at: String,
    pub reason: String,
}

/// Load tuned params from disk, if they exist.
pub async fn load_tuned_params() -> Option<TunedParams> {
    match fs::read_to_string(TUNED_PARAMS_PATH).await {
        Ok(contents) => serde_json::from_str(&contents).ok(),
        Err(_) => None,
    }
}

/// Save tuned params to disk.
pub async fn save_tuned_params(params: &TunedParams) {
    let dir = PathBuf::from(TUNED_PARAMS_PATH).parent().unwrap().to_path_buf();
    let _ = fs::create_dir_all(&dir).await;
    if let Ok(json) = serde_json::to_string_pretty(params) {
        let _ = fs::write(TUNED_PARAMS_PATH, json).await;
    }
}

/// Diagnose what went wrong and produce tuned parameters.
pub fn analyze_and_tune(ledger: &PerformanceLedger) -> (TunedParams, serde_json::Value) {
    let today = match ledger.today() {
        Some(t) => t,
        None => {
            let params = params_from_current();
            return (params, json!({"status": "no data to tune from"}));
        }
    };

    let yesterday = ledger.yesterday();
    let mut diagnosis: Vec<String> = Vec::new();
    let mut actions: Vec<String> = Vec::new();

    // Start from current params
    let mut p = params_from_strategy(&today.strategy_params);

    // ── Diagnosis 1: Too many trades (high fees eating profit) ──
    if today.total_fees > today.gross_pnl.abs() * 0.5 && today.total_trades > 20 {
        diagnosis.push(format!(
            "Fees (${:.4}) are >{:.0}% of gross P&L (${:.4}) — overtrading",
            today.total_fees, 50.0, today.gross_pnl
        ));
        // Raise entry bar, increase cooldown
        p.min_buy_signal = (p.min_buy_signal * 1.25).min(0.40);
        p.trade_cooldown_secs = (p.trade_cooldown_secs + 15).min(180);
        p.max_daily_trades = ((p.max_daily_trades as f64 * 0.75) as u32).max(15);
        actions.push("Raised MIN_BUY_SIGNAL, increased cooldown, lowered daily cap".into());
    }

    // ── Diagnosis 2: Avg hold too short (not capturing real moves) ──
    if today.avg_hold_seconds < 30 && today.total_trades > 10 {
        diagnosis.push(format!(
            "Avg hold {}s too short — exiting before moves develop",
            today.avg_hold_seconds
        ));
        p.flat_exit_secs = (p.flat_exit_secs + 60).min(600);
        p.sell_signal_threshold -= 0.03; // more negative = less trigger-happy
        p.sell_signal_threshold = p.sell_signal_threshold.max(-0.25);
        p.trailing_stop_pct = (p.trailing_stop_pct + 0.02).min(0.20);
        actions.push("Extended flat exit, loosened signal flip, widened trailing stop".into());
    }

    // ── Diagnosis 3: Win rate too low (<45%) ──
    if today.win_rate < 45.0 && today.total_trades > 10 {
        diagnosis.push(format!(
            "Win rate {:.1}% below 45% — entering bad trades",
            today.win_rate
        ));
        p.min_buy_signal = (p.min_buy_signal * 1.20).min(0.40);
        p.min_predicted_move_pct = (p.min_predicted_move_pct * 1.3).min(0.05);
        actions.push("Raised entry threshold and min predicted move".into());
    }

    // ── Diagnosis 4: Win rate high but avg profit too small ──
    if today.win_rate > 55.0 && today.avg_pnl_per_trade < 0.01 && today.total_trades > 10 {
        diagnosis.push(format!(
            "Good win rate ({:.1}%) but avg profit ${:.4} too small to cover spread",
            today.win_rate, today.avg_pnl_per_trade
        ));
        p.take_profit_pct = (p.take_profit_pct + 0.05).min(0.50);
        p.flat_exit_secs = (p.flat_exit_secs + 30).min(600);
        actions.push("Raised take profit target, extended hold time".into());
    }

    // ── Diagnosis 5: Too few trades (missed opportunities) ──
    if today.total_trades < 5 && today.gross_pnl < 0.10 {
        diagnosis.push(format!(
            "Only {} trades — entry thresholds may be too strict",
            today.total_trades
        ));
        p.min_buy_signal = (p.min_buy_signal * 0.85).max(0.05);
        p.trade_cooldown_secs = (p.trade_cooldown_secs.saturating_sub(10)).max(30);
        p.max_daily_trades = (p.max_daily_trades + 10).min(120);
        actions.push("Lowered MIN_BUY_SIGNAL, reduced cooldown, raised daily cap".into());
    }

    // ── Diagnosis 6: Negative net P&L ──
    if today.net_pnl < 0.0 {
        diagnosis.push(format!("Net P&L negative: ${:.4}", today.net_pnl));
        // Tighten hard stop to limit losses
        p.hard_stop_pct = (p.hard_stop_pct + 0.02).min(-0.08);
        actions.push("Tightened hard stop to limit downside".into());
    }

    // ── Diagnosis 7: Compared to yesterday — regression ──
    if let Some(yday) = yesterday {
        if today.net_pnl < yday.net_pnl && today.total_trades > 5 {
            diagnosis.push(format!(
                "Regression: today ${:.4} < yesterday ${:.4}",
                today.net_pnl, yday.net_pnl
            ));

            // If yesterday was better, blend toward yesterday's params
            let y = &yday.strategy_params;
            p.min_buy_signal = (p.min_buy_signal + y.min_buy_signal) / 2.0;
            p.sell_signal_threshold = (p.sell_signal_threshold + y.sell_signal_threshold) / 2.0;
            p.trailing_stop_pct = (p.trailing_stop_pct + y.trailing_stop_pct) / 2.0;
            p.trade_cooldown_secs = (p.trade_cooldown_secs + y.trade_cooldown_secs) / 2;
            p.flat_exit_secs = (p.flat_exit_secs + y.flat_exit_secs) / 2;
            actions.push("Blended toward yesterday's better-performing params".into());
        }
    }

    // No issues found
    if diagnosis.is_empty() {
        diagnosis.push("No significant issues detected — keeping current params".into());
    }

    let reason = format!("Diagnosis: {}. Actions: {}",
        diagnosis.join("; "),
        if actions.is_empty() { "none".to_string() } else { actions.join("; ") });

    p.tuned_at = chrono::Local::now().to_rfc3339();
    p.reason = reason.clone();

    let report = json!({
        "diagnosis": diagnosis,
        "actions": actions,
        "tuned_params": {
            "min_buy_signal": p.min_buy_signal,
            "strong_buy_signal": p.strong_buy_signal,
            "sell_signal_threshold": p.sell_signal_threshold,
            "trailing_stop_pct": p.trailing_stop_pct,
            "hard_stop_pct": p.hard_stop_pct,
            "take_profit_pct": p.take_profit_pct,
            "trade_cooldown_secs": p.trade_cooldown_secs,
            "flat_exit_secs": p.flat_exit_secs,
            "max_concurrent_positions": p.max_concurrent_positions,
            "max_daily_trades": p.max_daily_trades,
            "min_predicted_move_pct": p.min_predicted_move_pct,
        },
        "previous_params": today.strategy_params,
    });

    (p, report)
}

fn params_from_current() -> TunedParams {
    let sp = StrategyParams::current();
    params_from_strategy(&sp)
}

fn params_from_strategy(sp: &StrategyParams) -> TunedParams {
    TunedParams {
        min_buy_signal: sp.min_buy_signal,
        strong_buy_signal: sp.strong_buy_signal,
        sell_signal_threshold: sp.sell_signal_threshold,
        trailing_stop_pct: sp.trailing_stop_pct,
        hard_stop_pct: sp.hard_stop_pct,
        take_profit_pct: sp.take_profit_pct,
        trade_cooldown_secs: sp.trade_cooldown_secs,
        flat_exit_secs: sp.flat_exit_secs,
        max_concurrent_positions: sp.max_concurrent_positions,
        max_daily_trades: sp.max_daily_trades,
        min_predicted_move_pct: sp.min_predicted_move_pct,
        tuned_at: String::new(),
        reason: String::new(),
    }
}
