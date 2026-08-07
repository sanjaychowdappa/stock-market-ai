//! Daily sector-leader selection across a broad S&P 500 / NASDAQ universe.
//!
//! WHAT THIS IS
//! Rank every name in a sector by blended momentum, take that sector's leader,
//! then hold the leaders of the strongest sectors — at most one name per
//! sector, so five positions are five different bets rather than five
//! correlated ones.
//!
//! WHY IT LOOKS LIKE momentum_portfolio
//! Because it is the same method one level down. That module rotates between
//! sector ETFs (XLK, XLF, ...); this picks the best STOCK inside each sector.
//! The scoring is deliberately identical — blended 1/3/6-month momentum with an
//! absolute-momentum guard — so any difference in results is attributable to
//! stock-vs-ETF selection and nothing else. Inventing a new scoring rule here
//! would have made the comparison meaningless.
//!
//! WHY MOMENTUM AND NOT THE LIVE TRADER'S SIGNALS
//! Those signals have no demonstrated edge: exp1 lost to a random baseline over
//! 327 trades, and across this week's shadow books always_in_max_exposure
//! (+$67.60) and random_baseline (+$29.00) beat every signal-driven variant.
//! Applying them to 500 names instead of 5 would change which stocks get picked,
//! not whether the picking has skill. Cross-sectional momentum is the one
//! anomaly here with decades of out-of-sample support, so it is what this uses.
//!
//! WHY IT DOES NOT PLACE REAL ORDERS YET
//! A kill-criterion trial on the live trader started 2026-08-06 and its central
//! rule is that nothing changes mid-trial. This runs as its own measured paper
//! book against SPY, exactly as the ETF rotation does. If it earns its place,
//! switching to it is then a decision backed by evidence rather than a hope.
//!
//! A NOTE ON THE PREDECESSOR
//! An S&P 500 scanner existed and was deleted on 2026-08-05. It ranked a
//! universe and produced a top-5 list that NOTHING consumed — "Phase 2" was
//! never built, so it burned ~100 API calls a day feeding an endpoint nobody
//! read. This module has a paper book and a benchmark from its first commit
//! specifically so it cannot become that.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

const START_CASH: f64 = 3000.0;
/// One slot per sector covered. Not a tuned number — it is simply the sector
/// count, so the absolute-momentum filter decides how many names are actually
/// held. At 5 the agent ranked eleven sectors and threw six picks away.
const TOP_N: usize = 11;
const STATE_FILE: &str = "/app/reports/sector_leaders_state.json";
const LOG_FILE: &str = "/app/reports/sector_leaders.jsonl";
const BENCH_SYMBOL: &str = "SPY";

/// Same lookbacks as the ETF rotation: ~1, 3 and 6 months of trading days.
const MOM_WINDOWS: &[usize] = &[21, 63, 126];

/// Minimum blended momentum to be worth holding. A leader that is merely "least
/// bad in a falling sector" is not an opportunity — that slot stays in cash.
const MIN_ABSOLUTE_MOMENTUM: f64 = 0.0;

/// Liquid S&P 500 / NASDAQ names grouped by sector.
///
/// Deliberately ~5 per sector rather than the full 500: each name costs one
/// daily-bars request per scan, and the deleted scanner's ~100 calls/day for no
/// consumer is the failure being avoided. These are the most liquid names in
/// each GICS sector, which is where the tradeable volume is anyway. Dotted
/// tickers (BRK.B) are omitted — they break Alpaca's path parsing.
const SECTORS: &[(&str, &[&str])] = &[
    ("Technology",       &["AAPL", "MSFT", "NVDA", "AVGO", "AMD", "CRM", "ORCL"]),
    ("Communication",    &["GOOGL", "META", "NFLX", "DIS", "CMCSA"]),
    ("ConsumerDisc",     &["AMZN", "TSLA", "HD", "MCD", "NKE", "LOW"]),
    ("ConsumerStaples",  &["WMT", "COST", "PG", "KO", "PEP"]),
    ("Healthcare",       &["LLY", "UNH", "JNJ", "ABBV", "MRK", "TMO"]),
    ("Financials",       &["JPM", "V", "MA", "BAC", "WFC", "GS"]),
    ("Industrials",      &["CAT", "GE", "RTX", "UNP", "HON", "BA"]),
    ("Energy",           &["XOM", "CVX", "COP", "SLB"]),
    ("Utilities",        &["NEE", "DUK", "SO"]),
    ("Materials",        &["LIN", "SHW", "FCX"]),
    ("RealEstate",       &["PLD", "AMT", "EQIX"]),
];

#[derive(Default, Serialize, Deserialize)]
pub struct SectorLeaders {
    cash: f64,
    positions: HashMap<String, f64>, // symbol -> shares
    started_date: String,
    last_scan_date: String,
    scans: u32,
    rebalances: u32,
    bench_shares: f64,
    current_value: f64,
    bench_value: f64,
    /// Latest ranking: one entry per sector, best name first.
    leaders: Vec<LeaderPick>,
    holdings: Vec<String>,
    initialized: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LeaderPick {
    pub sector: String,
    pub symbol: String,
    pub momentum_pct: f64,
    pub price: f64,
    /// False when the sector's best name still has negative momentum.
    pub tradeable: bool,
}

impl SectorLeaders {
    pub fn to_json(&self) -> Value {
        let ret_pct = if self.initialized {
            (self.current_value - START_CASH) / START_CASH * 100.0
        } else { 0.0 };
        let bench_ret_pct = if self.bench_value > 0.0 {
            (self.bench_value - START_CASH) / START_CASH * 100.0
        } else { 0.0 };
        json!({
            "strategy": "daily sector-leader selection — best stock per sector by blended \
                         1/3/6-month momentum, max one name per sector",
            "universe_size": SECTORS.iter().map(|(_, s)| s.len()).sum::<usize>(),
            "sectors": SECTORS.len(),
            "max_positions": TOP_N,
            "benchmark": BENCH_SYMBOL,
            "paper_only": true,
            "why_paper_only": "The live trader is inside a kill-criterion trial started \
                               2026-08-06 whose rule is that nothing changes mid-trial. \
                               This proves itself on its own book first.",
            "initialized": self.initialized,
            "started_date": self.started_date,
            "last_scan_date": self.last_scan_date,
            "scans": self.scans,
            "rebalances": self.rebalances,
            "portfolio_value": (self.current_value * 100.0).round() / 100.0,
            "portfolio_return_pct": (ret_pct * 100.0).round() / 100.0,
            "benchmark_value": (self.bench_value * 100.0).round() / 100.0,
            "benchmark_return_pct": (bench_ret_pct * 100.0).round() / 100.0,
            "edge_vs_benchmark_pct": ((ret_pct - bench_ret_pct) * 100.0).round() / 100.0,
            "beating_benchmark": self.current_value > self.bench_value,
            "holdings": self.holdings,
            "sector_leaders": self.leaders,
        })
    }
}

pub type SharedSectorLeaders = Arc<Mutex<SectorLeaders>>;

pub fn create_shared() -> SharedSectorLeaders {
    let s = std::fs::read_to_string(STATE_FILE).ok()
        .and_then(|c| serde_json::from_str::<SectorLeaders>(&c).ok())
        .unwrap_or(SectorLeaders { cash: START_CASH, ..Default::default() });
    if s.initialized {
        info!("sector_leaders: restored (started {}, {} scans)", s.started_date, s.scans);
    }
    Arc::new(Mutex::new(s))
}

/// Mean of trailing returns over each lookback. Identical to the ETF module's
/// method so the two remain comparable.
fn blended_momentum(closes: &[f64]) -> Option<f64> {
    let n = closes.len();
    let last = *closes.last()?;
    let mut acc = 0.0;
    let mut used = 0;
    for &w in MOM_WINDOWS {
        if n > w {
            let past = closes[n - 1 - w];
            if past > 0.0 {
                acc += (last - past) / past * 100.0;
                used += 1;
            }
        }
    }
    if used == 0 { None } else { Some(acc / used as f64) }
}

static SCANNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Rank every sector's names, pick each sector's leader, hold the best TOP_N.
pub async fn scan(shared: &SharedSectorLeaders) {
    use std::sync::atomic::Ordering;
    // A scan issues ~50 sequential requests and can outlast its own interval;
    // overlapping runs would double the API load for nothing.
    if SCANNING.swap(true, Ordering::SeqCst) {
        warn!("sector_leaders: previous scan still running — skipping this cycle");
        return;
    }
    let result = scan_inner(shared).await;
    SCANNING.store(false, Ordering::SeqCst);
    result
}

async fn scan_inner(shared: &SharedSectorLeaders) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    info!("=== SECTOR LEADER SCAN ({} sectors, {} names) ===",
        SECTORS.len(), SECTORS.iter().map(|(_, s)| s.len()).sum::<usize>());

    let mut leaders: Vec<LeaderPick> = Vec::new();
    let mut prices: HashMap<String, f64> = HashMap::new();

    for (sector, names) in SECTORS {
        let mut best: Option<LeaderPick> = None;
        for &sym in *names {
            // 160 bars covers the 126-day window with room for holidays.
            let bars = match crate::services::alpaca_stream::fetch_daily_bars(sym, 160).await {
                Ok(b) => b,
                Err(e) => { warn!("  {}: bars failed ({})", sym, e); continue }
            };
            let closes: Vec<f64> = bars.iter().filter_map(|b| b["close"].as_f64()).collect();
            let price = match closes.last() { Some(p) => *p, None => continue };
            let mom = match blended_momentum(&closes) {
                Some(m) => m,
                None => { warn!("  {}: only {} bars — not enough history", sym, closes.len()); continue }
            };
            prices.insert(sym.to_string(), price);
            if best.as_ref().map(|b| mom > b.momentum_pct).unwrap_or(true) {
                best = Some(LeaderPick {
                    sector: sector.to_string(),
                    symbol: sym.to_string(),
                    momentum_pct: (mom * 100.0).round() / 100.0,
                    price: (price * 100.0).round() / 100.0,
                    tradeable: mom > MIN_ABSOLUTE_MOMENTUM,
                });
            }
        }
        if let Some(b) = best {
            info!("  {:<16} leader {:<6} momentum {:+.2}%{}",
                sector, b.symbol, b.momentum_pct,
                if b.tradeable { "" } else { "  (negative — not tradeable)" });
            leaders.push(b);
        }
    }

    // Strongest sectors first; only names with positive absolute momentum.
    leaders.sort_by(|a, b| b.momentum_pct.partial_cmp(&a.momentum_pct).unwrap());
    let picks: Vec<LeaderPick> = leaders.iter()
        .filter(|l| l.tradeable)
        .take(TOP_N)
        .cloned()
        .collect();

    if picks.is_empty() {
        info!("  no sector leader has positive momentum — holding cash");
    } else {
        info!("  HOLD: {}", picks.iter()
            .map(|p| format!("{} ({} {:+.2}%)", p.symbol, p.sector, p.momentum_pct))
            .collect::<Vec<_>>().join(", "));
    }

    // Benchmark: SPY bought once at inception and held.
    let spy_price = crate::services::alpaca_stream::fetch_daily_bars(BENCH_SYMBOL, 5).await
        .ok()
        .and_then(|b| b.last().and_then(|x| x["close"].as_f64()));

    let mut s = shared.lock();
    if !s.initialized {
        s.started_date = today.clone();
        s.cash = START_CASH;
        if let Some(p) = spy_price {
            if p > 0.0 { s.bench_shares = START_CASH / p; }
        }
        s.initialized = true;
    }

    // Rebalance only when the held SET changes. Re-ranking daily is free;
    // trading daily is not, and churn is what made the intraday trader lose.
    let new_set: Vec<String> = picks.iter().map(|p| p.symbol.clone()).collect();
    let changed = new_set != s.holdings;
    if changed && !picks.is_empty() {
        let total = s.cash + s.positions.iter()
            .filter_map(|(sym, sh)| prices.get(sym).map(|p| p * sh))
            .sum::<f64>();
        let per = total / picks.len() as f64;
        s.positions.clear();
        for p in &picks {
            if p.price > 0.0 { s.positions.insert(p.symbol.clone(), per / p.price); }
        }
        s.cash = 0.0;
        s.rebalances += 1;
        info!("  rebalanced #{} — ${:.2} across {} names", s.rebalances, total, picks.len());
    }

    s.current_value = s.cash + s.positions.iter()
        .filter_map(|(sym, sh)| prices.get(sym).map(|p| p * sh))
        .sum::<f64>();
    if let Some(p) = spy_price { s.bench_value = s.bench_shares * p; }
    s.holdings = new_set;
    s.leaders = leaders;
    s.last_scan_date = today.clone();
    s.scans += 1;

    let snapshot = s.to_json();
    if let Ok(txt) = serde_json::to_string(&s.to_json()) {
        let _ = std::fs::write(STATE_FILE, txt);
    }
    drop(s);

    // Append a daily row so the record survives independently of state.
    if let Ok(line) = serde_json::to_string(&json!({
        "date": today,
        "value": snapshot["portfolio_value"],
        "benchmark": snapshot["benchmark_value"],
        "edge_pct": snapshot["edge_vs_benchmark_pct"],
        "holdings": snapshot["holdings"],
    })) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
            let _ = writeln!(f, "{}", line);
        }
    }
    info!("=== SECTOR LEADER SCAN COMPLETE ===");
}

/// Daily scan, shortly after the open so the momentum uses a settled price.
pub fn spawn(shared: SharedSectorLeaders) {
    tokio::spawn(async move {
        // Stagger against the other startup tasks so ~50 requests do not land
        // while Kronos and the institutional signals are also fetching.
        tokio::time::sleep(tokio::time::Duration::from_secs(120)).await;
        loop {
            scan(&shared).await;
            tokio::time::sleep(tokio::time::Duration::from_secs(86_400)).await;
        }
    });
}

pub static _UNUSED: Lazy<()> = Lazy::new(|| ());
