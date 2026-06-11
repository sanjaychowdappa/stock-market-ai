//! End-of-day results publisher.
//!
//! At market close (4:05 PM ET), captures the full paper trading session
//! and writes a JSON report + human-readable summary to /app/reports/.
//! Also exposes a REST endpoint for on-demand report generation.

use crate::config::*;
use crate::services::paper_trader::PaperTrader;
use crate::services::daily_tracker::DailyTracker;
use chrono::{Local, Timelike};
use serde_json::json;
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

const REPORTS_DIR: &str = "/app/reports";

/// Generate the full EOD report from current trader + tracker state.
pub fn generate_report(
    trader: &PaperTrader,
    tracker: &DailyTracker,
) -> serde_json::Value {
    let now = Local::now();
    let payload = trader.build_payload();
    let tracker_status = tracker.status();

    let portfolio = &payload["portfolio"];
    let stats = &payload["stats"];

    // Build from symbols data
    let symbols_summary: Vec<serde_json::Value> = if let Some(syms) = payload["symbols"].as_array() {
        syms.iter().map(|s| {
            json!({
                "symbol": s["symbol"],
                "last_price": s["price"],
                "direction": s["direction"],
                "signal_strength": s["signal"],
                "in_position": s["in_position"],
                "position_pnl": s["position_pnl"],
            })
        }).collect()
    } else {
        Vec::new()
    };

    // Data quality from tracker
    let data_collected = &tracker_status["data_collected"];

    json!({
        "report": {
            "type": "end_of_day",
            "generated_at": now.to_rfc3339(),
            "date": now.format("%Y-%m-%d").to_string(),
            "market_session": {
                "open": "09:30 ET",
                "close": "16:00 ET",
                "status": if now.hour() >= 16 { "closed" } else if now.hour() >= 9 && now.minute() >= 30 { "open" } else { "pre-market" },
            },
        },
        "portfolio": {
            "initial_capital": INITIAL_CASH,
            "cash": portfolio["cash"],
            "positions_value": portfolio["positions_value"],
            "total_value": portfolio["total_value"],
            "total_pnl": portfolio["total_pnl"],
            "total_pnl_pct": portfolio["total_pnl_pct"],
            "realized_pnl": portfolio["realized_pnl"],
            "drawdown_pct": portfolio["drawdown_pct"],
            "target": {
                "goal": "$500 → $600 challenge",
                "target_value": portfolio["target_value"],
                "progress_pct": portfolio["progress_pct"],
            },
        },
        "trading_stats": {
            "total_trades": stats["total_trades"],
            "winning_trades": stats["winning_trades"],
            "win_rate": stats["win_rate"],
            "avg_hold_seconds": stats["avg_hold_seconds"],
            "open_positions": stats["open_positions"],
            "uptime_seconds": stats["uptime_seconds"],
        },
        "symbols": symbols_summary,
        "positions": payload["positions"],
        "recent_trades": payload["recent_trades"],
        "value_history": payload["value_history"],
        "data_quality": {
            "tracker": data_collected,
            "finetune_triggered": tracker_status["finetune_triggered"],
            "snapshots": tracker_status["snapshots_count"],
        },
    })
}

/// Generate a human-readable text summary.
pub fn generate_text_summary(report: &serde_json::Value) -> String {
    let date = report["report"]["date"].as_str().unwrap_or("unknown");
    let generated = report["report"]["generated_at"].as_str().unwrap_or("unknown");

    let total_value = report["portfolio"]["total_value"].as_f64().unwrap_or(0.0);
    let total_pnl = report["portfolio"]["total_pnl"].as_f64().unwrap_or(0.0);
    let total_pnl_pct = report["portfolio"]["total_pnl_pct"].as_f64().unwrap_or(0.0);
    let realized = report["portfolio"]["realized_pnl"].as_f64().unwrap_or(0.0);
    let cash = report["portfolio"]["cash"].as_f64().unwrap_or(0.0);
    let positions_val = report["portfolio"]["positions_value"].as_f64().unwrap_or(0.0);
    let progress = report["portfolio"]["target"]["progress_pct"].as_f64().unwrap_or(0.0);

    let total_trades = report["trading_stats"]["total_trades"].as_u64().unwrap_or(0);
    let winning = report["trading_stats"]["winning_trades"].as_u64().unwrap_or(0);
    let win_rate = report["trading_stats"]["win_rate"].as_f64().unwrap_or(0.0);
    let avg_hold = report["trading_stats"]["avg_hold_seconds"].as_u64().unwrap_or(0);
    let uptime = report["trading_stats"]["uptime_seconds"].as_u64().unwrap_or(0);

    let uptime_h = uptime / 3600;
    let uptime_m = (uptime % 3600) / 60;

    let pnl_emoji = if total_pnl > 0.0 { "+" } else { "" };

    let mut s = String::new();
    s.push_str("╔══════════════════════════════════════════════════════════╗\n");
    s.push_str("║           STOCK MARKET AI — END OF DAY REPORT           ║\n");
    s.push_str("╚══════════════════════════════════════════════════════════╝\n\n");

    s.push_str(&format!("  Date:       {}\n", date));
    s.push_str(&format!("  Generated:  {}\n", generated));
    s.push_str(&format!("  Uptime:     {}h {}m\n\n", uptime_h, uptime_m));

    s.push_str("─────────────── PORTFOLIO ───────────────\n\n");
    s.push_str(&format!("  Initial Capital:  ${:.2}\n", INITIAL_CASH));
    s.push_str(&format!("  Current Value:    ${:.2}\n", total_value));
    s.push_str(&format!("  Cash:             ${:.2}\n", cash));
    s.push_str(&format!("  Positions Value:  ${:.2}\n", positions_val));
    s.push_str(&format!("  Total P&L:        {}{:.4} ({}{:.2}%)\n", pnl_emoji, total_pnl, pnl_emoji, total_pnl_pct));
    s.push_str(&format!("  Realized P&L:     ${:.4}\n", realized));
    s.push_str(&format!("  Challenge:        $100 → $150 ({:.1}% complete)\n\n", progress));

    s.push_str("─────────────── TRADING STATS ───────────────\n\n");
    s.push_str(&format!("  Total Trades:     {}\n", total_trades));
    s.push_str(&format!("  Winning Trades:   {}\n", winning));
    s.push_str(&format!("  Win Rate:         {:.1}%\n", win_rate));
    s.push_str(&format!("  Avg Hold Time:    {}s\n\n", avg_hold));

    s.push_str("─────────────── SYMBOLS ───────────────\n\n");
    if let Some(symbols) = report["symbols"].as_array() {
        s.push_str("  Symbol   Price       Direction    Signal   Position\n");
        s.push_str("  ──────   ─────────   ─────────   ──────   ────────\n");
        for sym in symbols {
            let name = sym["symbol"].as_str().unwrap_or("?");
            let price = sym["last_price"].as_f64().unwrap_or(0.0);
            let dir = sym["direction"].as_str().unwrap_or("?");
            let sig = sym["signal_strength"].as_f64().unwrap_or(0.0);
            let in_pos = sym["in_position"].as_bool().unwrap_or(false);
            s.push_str(&format!("  {:<7}  ${:<9.2}  {:<10}  {:<7.3}  {}\n",
                name, price, dir, sig, if in_pos { "OPEN" } else { "—" }));
        }
    }
    s.push('\n');

    // Open positions detail
    if let Some(positions) = report["positions"].as_array() {
        if !positions.is_empty() {
            s.push_str("─────────────── OPEN POSITIONS ───────────────\n\n");
            for p in positions {
                let sym = p["symbol"].as_str().unwrap_or("?");
                let shares = p["shares"].as_f64().unwrap_or(0.0);
                let entry = p["entry_price"].as_f64().unwrap_or(0.0);
                let current = p["current_price"].as_f64().unwrap_or(0.0);
                let pnl = p["unrealized_pnl"].as_f64().unwrap_or(0.0);
                let pnl_pct = p["unrealized_pnl_pct"].as_f64().unwrap_or(0.0);
                s.push_str(&format!("  {} — {:.4} shares @ ${:.2} → ${:.2}  P&L: ${:.4} ({:.2}%)\n",
                    sym, shares, entry, current, pnl, pnl_pct));
            }
            s.push('\n');
        }
    }

    // Recent trades
    if let Some(trades) = report["recent_trades"].as_array() {
        if !trades.is_empty() {
            s.push_str("─────────────── RECENT TRADES (last 20) ───────────────\n\n");
            s.push_str("  Time      Action  Symbol   Price       Total    P&L       Reason\n");
            s.push_str("  ────────  ──────  ──────   ─────────   ──────   ─────     ──────\n");
            for t in trades {
                let time = t["time"].as_str().unwrap_or("?");
                let action = t["action"].as_str().unwrap_or("?");
                let sym = t["symbol"].as_str().unwrap_or("?");
                let price = t["price"].as_f64().unwrap_or(0.0);
                let total = t["total"].as_f64().unwrap_or(0.0);
                let pnl = t["pnl"].as_f64().unwrap_or(0.0);
                let reason = t["reason"].as_str().unwrap_or("");
                s.push_str(&format!("  {:<9} {:<6} {:<7}  ${:<9.2} ${:<6.2}  ${:<8.4}  {}\n",
                    time, action, sym, price, total, pnl, reason));
            }
            s.push('\n');
        }
    }

    s.push_str("═══════════════════════════════════════════════════════════\n");
    s.push_str("  Powered by Stock Market AI — Rust Engine + Kronos ONNX\n");
    s.push_str("═══════════════════════════════════════════════════════════\n");

    s
}

/// Save report to disk.
pub async fn save_report(report: &serde_json::Value, text_summary: &str) -> Result<PathBuf, String> {
    let date = report["report"]["date"].as_str().unwrap_or("unknown");
    let dir = PathBuf::from(REPORTS_DIR);

    if let Err(e) = fs::create_dir_all(&dir).await {
        return Err(format!("Failed to create reports dir: {}", e));
    }

    let json_path = dir.join(format!("eod_report_{}.json", date));
    let txt_path = dir.join(format!("eod_report_{}.txt", date));

    // Save JSON
    if let Err(e) = fs::write(&json_path, serde_json::to_string_pretty(report).unwrap()).await {
        return Err(format!("Failed to write JSON report: {}", e));
    }

    // Save text
    if let Err(e) = fs::write(&txt_path, text_summary).await {
        return Err(format!("Failed to write text report: {}", e));
    }

    info!("EOD report saved: {:?} + {:?}", json_path, txt_path);
    Ok(dir)
}
