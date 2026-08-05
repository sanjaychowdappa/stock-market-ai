//! Daily Stock Picker — uses Kronos to rank stocks at market open.
//!
//! Runs Kronos once at startup and once at midday to determine which
//! stocks to focus on for the day. The paper trader only trades stocks
//! that Kronos ranks as bullish.
//!
//! This solves the Kronos oscillation problem: instead of using Kronos
//! every 8 seconds (which flips bullish/bearish randomly), we use it
//! once per session with full historical data for a stable daily forecast.

use crate::config::TOP_SYMBOLS;
use crate::services::alpaca_stream;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

/// A stock's daily ranking from Kronos.
#[derive(Debug, Clone)]
pub struct StockRanking {
    pub symbol: String,
    pub predicted_change_pct: f64,
    pub direction: String, // "bullish" or "bearish"
    pub should_trade: bool,
    pub allocation_weight: f64, // 0.0 to 1.0 — how much capital to allocate
    pub model_source: String,
}

/// The daily focus list — which stocks to trade and how much.
#[derive(Debug, Clone)]
pub struct DailyFocus {
    pub rankings: Vec<StockRanking>,
    pub focus_symbols: Vec<String>,    // Only these get traded
    pub last_updated: String,
    pub update_count: u32,
}

impl DailyFocus {
    pub fn new() -> Self {
        // Default: trade all symbols until Kronos provides ranking
        Self {
            rankings: TOP_SYMBOLS.iter().map(|&s| StockRanking {
                symbol: s.to_string(),
                predicted_change_pct: 0.0,
                direction: "neutral".to_string(),
                should_trade: true,
                allocation_weight: 1.0 / TOP_SYMBOLS.len() as f64,
                model_source: "default".to_string(),
            }).collect(),
            focus_symbols: TOP_SYMBOLS.iter().map(|s| s.to_string()).collect(),
            last_updated: "not yet".to_string(),
            update_count: 0,
        }
    }

    /// Check if a symbol is in today's focus list.
    pub fn is_tradeable(&self, symbol: &str) -> bool {
        self.focus_symbols.iter().any(|s| s == symbol)
    }

    /// Get allocation weight for a symbol (0.0 if not in focus).
    pub fn get_weight(&self, symbol: &str) -> f64 {
        self.rankings.iter()
            .find(|r| r.symbol == symbol)
            .map(|r| if r.should_trade { r.allocation_weight } else { 0.0 })
            .unwrap_or(0.0)
    }

    /// Get the ranking info as JSON for display.
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "last_updated": self.last_updated,
            "update_count": self.update_count,
            "focus_symbols": self.focus_symbols,
            "rankings": self.rankings.iter().map(|r| json!({
                "symbol": r.symbol,
                "predicted_change_pct": format!("{:.3}%", r.predicted_change_pct),
                "direction": r.direction,
                "should_trade": r.should_trade,
                "allocation_weight": format!("{:.0}%", r.allocation_weight * 100.0),
                "model": r.model_source,
            })).collect::<Vec<_>>(),
        })
    }
}

pub type SharedDailyFocus = Arc<Mutex<DailyFocus>>;

pub fn create_shared() -> SharedDailyFocus {
    Arc::new(Mutex::new(DailyFocus::new()))
}

/// Run Kronos ranking for all symbols. Call at market open and midday.
pub async fn run_kronos_ranking(focus: &SharedDailyFocus) {
    info!("=== KRONOS DAILY STOCK RANKING ===");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let sidecar_url = "http://finetune-sidecar:8001";

    // Check sidecar health
    match client.get(format!("{}/health", sidecar_url)).send().await {
        Ok(resp) if resp.status().is_success() => {}
        _ => {
            warn!("Kronos sidecar not available — keeping all symbols tradeable");
            return;
        }
    }

    let mut rankings: Vec<StockRanking> = Vec::new();

    for &sym in TOP_SYMBOLS {
        info!("  Analyzing {}...", sym);

        // Fetch historical bars from Alpaca
        match alpaca_stream::fetch_historical_bars(sym, 500).await {
            Ok(candle_data) if candle_data.len() >= 30 => {
                let body = json!({
                    "symbol": sym,
                    "candles": candle_data,
                    "steps": 5,
                });

                match client.post(format!("{}/predict", sidecar_url))
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            if data.get("error").is_none() {
                                let change = data["total_change_pct"].as_f64().unwrap_or(0.0);
                                let direction = data["direction"].as_str().unwrap_or("neutral").to_string();
                                let source = data["model_source"].as_str().unwrap_or("unknown").to_string();

                                info!("  {}: Kronos predicts {} ({:+.3}%) [{}]",
                                    sym, direction, change, source);

                                rankings.push(StockRanking {
                                    symbol: sym.to_string(),
                                    predicted_change_pct: change,
                                    direction,
                                    should_trade: false, // Set below
                                    allocation_weight: 0.0,
                                    model_source: source,
                                });
                                continue;
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Fallback: neutral ranking
        warn!("  {}: Could not get Kronos prediction — marking neutral", sym);
        rankings.push(StockRanking {
            symbol: sym.to_string(),
            predicted_change_pct: 0.0,
            direction: "neutral".to_string(),
            should_trade: true, // Trade by default if no data
            allocation_weight: 0.2,
            model_source: "fallback".to_string(),
        });
    }

    // Sort by predicted change (most bullish first)
    rankings.sort_by(|a, b| b.predicted_change_pct.partial_cmp(&a.predicted_change_pct).unwrap());

    // Select focus stocks: top 2-3 bullish, skip bearish
    let mut focus_symbols: Vec<String> = Vec::new();
    let mut total_weight = 0.0;

    for r in rankings.iter_mut() {
        if r.predicted_change_pct > 0.0 {
            // Bullish — trade it
            r.should_trade = true;
            // Weight proportional to predicted gain (normalized later)
            r.allocation_weight = r.predicted_change_pct.abs().max(0.01);
            total_weight += r.allocation_weight;
            focus_symbols.push(r.symbol.clone());
        } else if r.predicted_change_pct > -0.05 {
            // Slightly bearish / neutral — small allocation allowed
            r.should_trade = true;
            r.allocation_weight = 0.05; // Minimal weight
            total_weight += r.allocation_weight;
            focus_symbols.push(r.symbol.clone());
        } else {
            // Clearly bearish — skip
            r.should_trade = false;
            r.allocation_weight = 0.0;
        }
    }

    // Normalize weights to sum to 1.0
    if total_weight > 0.0 {
        for r in rankings.iter_mut() {
            if r.should_trade {
                r.allocation_weight /= total_weight;
            }
        }
    }

    // If no stocks are bullish, trade top 2 anyway (least bearish)
    if focus_symbols.is_empty() {
        warn!("  No bullish stocks! Using top 2 least bearish");
        for r in rankings.iter_mut().take(2) {
            r.should_trade = true;
            r.allocation_weight = 0.5;
            focus_symbols.push(r.symbol.clone());
        }
    }

    let now = chrono::Local::now().format("%H:%M:%S").to_string();

    info!("  ─── DAILY FOCUS LIST ───");
    for r in &rankings {
        let tag = if r.should_trade {
            format!("TRADE ({:.0}%)", r.allocation_weight * 100.0)
        } else {
            "SKIP".to_string()
        };
        info!("  {} {:+.3}% {} → {}", r.symbol, r.predicted_change_pct, r.direction, tag);
    }
    info!("  Focus: {:?}", focus_symbols);
    info!("=== RANKING COMPLETE ===");

    // Update shared state
    let mut f = focus.lock();
    f.rankings = rankings;
    f.focus_symbols = focus_symbols;
    f.last_updated = now;
    f.update_count += 1;
}

// ════════════════════════════════════════════════════════════════
//  S&P 500 DAILY SCANNER (Phase 1)
//

// ════════════════════════════════════════════════════════════════
//  The S&P 500 scanner that lived here is removed.
//
//  Its auto-run was disabled in the 2026-07-29 audit because nothing
//  consumed the output — "Phase 2" (wiring picks into live trading) was
//  never built. What remained was a 130-line universe + scan routine that
//  no caller invoked, and an /api/scan endpoint serving a cache that was
//  therefore always empty. Dead weight that read as a working feature.
//
//  Kronos daily ranking (run_kronos_ranking above) is unaffected and still
//  produces the top-5 focus list the trader actually uses.
// ════════════════════════════════════════════════════════════════
