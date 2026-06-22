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
use serde_json::{json, Value};
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
//  Ranks a broad liquid universe with Kronos on DAILY bars to find the
//  best multi-day swing candidates. Read-only: it produces a ranked
//  top-5 watchlist. Phase 2 will wire the picks into the live trader.
// ════════════════════════════════════════════════════════════════

/// Liquid S&P 500 universe for the daily scanner. Seeded with the most-liquid
/// large-caps (covers the bulk of index volume); expand toward the full 500
/// by adding tickers here. Dotted tickers (BRK.B) omitted to keep fetches clean.
pub const SP500_UNIVERSE: &[&str] = &[
    "AAPL","MSFT","NVDA","AMZN","GOOGL","GOOG","META","TSLA","AVGO","LLY",
    "JPM","V","UNH","XOM","MA","COST","HD","PG","JNJ","WMT",
    "NFLX","BAC","CRM","ORCL","MRK","ABBV","CVX","KO","AMD","PEP",
    "ADBE","TMO","LIN","ACN","MCD","CSCO","WFC","ABT","DHR","TXN",
    "INTC","QCOM","INTU","AMAT","IBM","PM","CAT","GE","VZ","NOW",
    "AXP","PFE","UNP","GS","MS","RTX","HON","NEE","LOW","COP",
    "BKNG","UBER","T","SPGI","BA","PLD","ELV","SCHW","BLK","SBUX",
    "MDT","GILD","DE","ADP","LMT","CB","MMC","C","BMY","AMT",
    "MU","ADI","SO","DUK","CI","REGN","MO","BSX","TJX","ETN",
    "ISRG","VRTX","ZTS","PGR","SLB","EQIX","PANW","KLAC","SNPS","CDNS",
];

/// A single ranked pick from the daily scan.
#[derive(Debug, Clone)]
pub struct ScanPick {
    pub symbol: String,
    pub predicted_change_pct: f64,
    pub direction: String,
}

/// Result of the latest S&P 500 daily scan.
#[derive(Debug, Clone, Default)]
pub struct Sp500Scan {
    pub top_picks: Vec<ScanPick>,
    pub scanned: usize,
    pub universe: usize,
    pub last_updated: String,
    pub horizon_days: u32,
    pub running: bool,
}

impl Sp500Scan {
    pub fn to_json(&self) -> Value {
        json!({
            "last_updated": self.last_updated,
            "running": self.running,
            "horizon_days": self.horizon_days,
            "universe_size": self.universe,
            "scanned": self.scanned,
            "top_picks": self.top_picks.iter().map(|p| json!({
                "symbol": p.symbol,
                "predicted_change_pct": format!("{:+.3}%", p.predicted_change_pct),
                "direction": p.direction,
            })).collect::<Vec<_>>(),
        })
    }
}

pub type SharedSp500Scan = Arc<Mutex<Sp500Scan>>;

pub fn create_scan_shared() -> SharedSp500Scan {
    Arc::new(Mutex::new(Sp500Scan::default()))
}

/// Scan the S&P 500 universe with Kronos on daily bars; rank by predicted
/// multi-day change and store the top 5. Runs in the background (~once/day).
pub async fn run_sp500_scan(scan: &SharedSp500Scan) {
    info!("=== S&P 500 KRONOS SCAN (daily bars, multi-day horizon) ===");
    scan.lock().running = true;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();
    let sidecar_url = "http://finetune-sidecar:8001";

    match client.get(format!("{}/health", sidecar_url)).send().await {
        Ok(resp) if resp.status().is_success() => {}
        _ => {
            warn!("Kronos sidecar unavailable — skipping S&P 500 scan");
            scan.lock().running = false;
            return;
        }
    }

    let horizon = 3u32; // predict ~3 trading days ahead
    let mut picks: Vec<ScanPick> = Vec::new();
    let mut scanned = 0usize;
    let mut fetch_fail = 0usize;
    let mut kronos_fail = 0usize;

    for &sym in SP500_UNIVERSE {
        // Throttle to stay under Alpaca's free-tier rate limit (~200/min).
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;

        let candles = match alpaca_stream::fetch_daily_bars(sym, 120).await {
            Ok(c) if c.len() >= 30 => c,
            Ok(c) => { fetch_fail += 1; if fetch_fail <= 3 { warn!("  {} scan: only {} daily bars", sym, c.len()); } continue; }
            Err(e) => { fetch_fail += 1; if fetch_fail <= 3 { warn!("  {} scan fetch err: {}", sym, e); } continue; }
        };

        let body = json!({ "symbol": sym, "candles": candles, "steps": horizon });
        match client.post(format!("{}/predict", sidecar_url)).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<Value>().await {
                    if data.get("error").is_none() {
                        let change = data["total_change_pct"].as_f64().unwrap_or(0.0);
                        let dir = data["direction"].as_str().unwrap_or("neutral").to_string();
                        picks.push(ScanPick { symbol: sym.to_string(), predicted_change_pct: change, direction: dir });
                        scanned += 1;
                    } else { kronos_fail += 1; if kronos_fail <= 3 { warn!("  {} kronos error: {:?}", sym, data.get("error")); } }
                } else { kronos_fail += 1; }
            }
            Ok(resp) => { kronos_fail += 1; if kronos_fail <= 3 { warn!("  {} kronos http {}", sym, resp.status()); } }
            Err(e) => { kronos_fail += 1; if kronos_fail <= 3 { warn!("  {} kronos req err: {}", sym, e); } }
        }
    }
    info!("  scan tally: {} ok, {} fetch-fail, {} kronos-fail", scanned, fetch_fail, kronos_fail);

    picks.sort_by(|a, b| b.predicted_change_pct
        .partial_cmp(&a.predicted_change_pct)
        .unwrap_or(std::cmp::Ordering::Equal));
    let top: Vec<ScanPick> = picks.iter().take(5).cloned().collect();

    info!("  S&P 500 scan complete: {}/{} scanned. Top 5 picks (next ~{}d):",
        scanned, SP500_UNIVERSE.len(), horizon);
    for p in &top {
        info!("    {} {:+.3}% ({})", p.symbol, p.predicted_change_pct, p.direction);
    }
    info!("=== S&P 500 SCAN COMPLETE ===");

    let mut s = scan.lock();
    s.top_picks = top;
    s.scanned = scanned;
    s.universe = SP500_UNIVERSE.len();
    s.horizon_days = horizon;
    s.last_updated = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    s.running = false;
}
