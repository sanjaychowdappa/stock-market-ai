"""
Paper Trading Script: Compare Kronos predictions vs actual NVDA data.
Also runs backtesting on recent historical data.
"""
import sys
sys.path.insert(0, "/app/kronos_repo")

import yfinance as yf
import pandas as pd
import numpy as np
import json
from datetime import datetime, timedelta

# ── 1. Fetch actual NVDA data ──────────────────────────────────────
print("=" * 70)
print("NVIDIA (NVDA) PAPER TRADING REPORT")
print(f"Report Date: {datetime.now().strftime('%Y-%m-%d %H:%M')}")
print("=" * 70)

ticker = yf.Ticker("NVDA")

# Today's live intraday
df_today = ticker.history(period="1d", interval="1m")
if not df_today.empty:
    today_str = datetime.now().strftime("%Y-%m-%d")
    today_open = float(df_today.iloc[0]["Open"])
    today_high = float(df_today["High"].max())
    today_low = float(df_today["Low"].min())
    today_close = float(df_today.iloc[-1]["Close"])
    today_vol = int(df_today["Volume"].sum())
    candle_count = len(df_today)

    print(f"\n[LIVE] Today ({today_str}) — {candle_count} candles so far")
    print(f"  Open:   ${today_open:.2f}")
    print(f"  High:   ${today_high:.2f}")
    print(f"  Low:    ${today_low:.2f}")
    print(f"  Close:  ${today_close:.2f}  (current)")
    print(f"  Volume: {today_vol:,}")
    chg = today_close - today_open
    chg_pct = (chg / today_open) * 100
    arrow = "▲" if chg >= 0 else "▼"
    print(f"  Change: {arrow} {chg:+.2f} ({chg_pct:+.2f}%)")
else:
    print("\n[!] Market may be closed — no intraday data available")
    today_close = None

# Last 30 days daily for backtesting
df_daily = ticker.history(period="3mo", interval="1d")
print(f"\n{'─' * 70}")
print("RECENT DAILY HISTORY (Last 10 days)")
print(f"{'─' * 70}")
print(f"{'Date':<12} {'Open':>8} {'High':>8} {'Low':>8} {'Close':>8} {'Volume':>12} {'Chg%':>7}")
print("-" * 70)

last10 = df_daily.tail(10)
for idx, row in last10.iterrows():
    d = idx.strftime("%Y-%m-%d")
    prev_close = df_daily.loc[:idx].iloc[-2]["Close"] if len(df_daily.loc[:idx]) > 1 else row["Open"]
    chg_pct = ((row["Close"] - prev_close) / prev_close) * 100
    print(f"{d:<12} {row['Open']:>8.2f} {row['High']:>8.2f} {row['Low']:>8.2f} {row['Close']:>8.2f} {int(row['Volume']):>12,} {chg_pct:>+6.2f}%")

# ── 2. Run Kronos Predictions (Backtest on known data) ─────────────
print(f"\n{'=' * 70}")
print("KRONOS BACKTEST — Predicting known days from historical data")
print(f"{'=' * 70}")

from app.services.kronos_predictor import _load_kronos, kronos_predict
import app.services.kronos_predictor as kp

# Load model
loaded = _load_kronos()
if not loaded:
    print(f"[ERROR] Could not load Kronos model: {kp._load_error}")
    sys.exit(1)

print(f"[OK] Kronos model loaded on {kp._predictor.device}")

# Prepare full daily data
df_full = df_daily.reset_index()
df_full.rename(columns={"Date": "Date", "Datetime": "Date"}, inplace=True)
date_col = "Date" if "Date" in df_full.columns else df_full.columns[0]
df_full.rename(columns={date_col: "Date"}, inplace=True)
df_full["Date"] = pd.to_datetime(df_full["Date"]).dt.strftime("%Y-%m-%d")

# Backtest: use data up to N days ago, predict next 5 days, compare to actual
backtest_days = [5, 10, 15, 20]
all_errors = {"open": [], "high": [], "low": [], "close": []}

for cutoff in backtest_days:
    if cutoff + 5 > len(df_full):
        continue

    train_df = df_full.iloc[:-cutoff].copy()
    actual_df = df_full.iloc[-cutoff:-cutoff+5].copy() if cutoff > 5 else df_full.iloc[-cutoff:].copy()
    steps = min(5, len(actual_df))

    if len(train_df) < 50:
        continue

    result = kronos_predict(train_df, steps=steps)
    if "error" in result:
        print(f"\n[Backtest cutoff={cutoff}d] ERROR: {result['error']}")
        continue

    print(f"\n[Backtest] Predicting {steps} days from {cutoff} days ago")
    print(f"  Training data: {len(train_df)} days, ending {train_df['Date'].iloc[-1]}")
    print(f"  {'Day':<6} {'Metric':<8} {'Predicted':>10} {'Actual':>10} {'Error':>10} {'Error%':>8}")
    print(f"  {'-' * 55}")

    preds = result["predictions"]
    for i in range(min(len(preds), len(actual_df))):
        p = preds[i]
        a = actual_df.iloc[i]
        for metric, pred_key, act_key in [
            ("Open", "predicted_open", "Open"),
            ("High", "predicted_high", "High"),
            ("Low", "predicted_low", "Low"),
            ("Close", "predicted_close", "Close"),
        ]:
            pred_val = p[pred_key]
            act_val = float(a[act_key])
            err = pred_val - act_val
            err_pct = (err / act_val) * 100
            all_errors[metric.lower()].append(abs(err_pct))
            print(f"  D{i+1:<5} {metric:<8} ${pred_val:>9.2f} ${act_val:>9.2f} {err:>+9.2f} {err_pct:>+7.2f}%")

# ── 3. Error Summary ──────────────────────────────────────────────
print(f"\n{'=' * 70}")
print("PREDICTION ERROR SUMMARY (Absolute % Error)")
print(f"{'=' * 70}")
for metric in ["open", "high", "low", "close"]:
    errors = all_errors[metric]
    if errors:
        print(f"  {metric.upper():<6}  Mean: {np.mean(errors):.2f}%  Median: {np.median(errors):.2f}%  Max: {np.max(errors):.2f}%  Samples: {len(errors)}")

overall = []
for v in all_errors.values():
    overall.extend(v)
if overall:
    print(f"\n  OVERALL  Mean: {np.mean(overall):.2f}%  Median: {np.median(overall):.2f}%")

# ── 4. Today's Forward Prediction ──────────────────────────────────
print(f"\n{'=' * 70}")
print("KRONOS FORWARD PREDICTION (Next 7 Trading Days)")
print(f"{'=' * 70}")

result = kronos_predict(df_full, steps=7)
if "error" not in result:
    print(f"  Current Close: ${result['current_close']}")
    print(f"  7-Day Target:  ${result['final_predicted_close']}  ({result['total_change_percent']:+.2f}%)")
    print(f"  Direction:     {result['direction'].upper()}")
    print()
    print(f"  {'Day':<6} {'Date':<12} {'Open':>8} {'High':>8} {'Low':>8} {'Close':>8} {'Chg%':>7} {'Dir':<8}")
    print(f"  {'-' * 65}")
    for p in result["predictions"]:
        print(f"  D{p['step']:<5} {p['date']:<12} ${p['predicted_open']:>7.2f} ${p['predicted_high']:>7.2f} ${p['predicted_low']:>7.2f} ${p['predicted_close']:>7.2f} {p['change_percent']:>+6.2f}% {p['direction']}")
else:
    print(f"  ERROR: {result['error']}")

# ── 5. Paper Trading Simulation ────────────────────────────────────
print(f"\n{'=' * 70}")
print("PAPER TRADING SIMULATION")
print(f"{'=' * 70}")

initial_capital = 100000
shares = 0
cash = initial_capital
trades = []

# Use Kronos backtest predictions to simulate trades on last 20 days
df_trade = df_full.tail(25).copy().reset_index(drop=True)

for i in range(5, len(df_trade) - 1):
    train_slice = df_trade.iloc[:i].copy()
    next_day = df_trade.iloc[i]

    pred = kronos_predict(train_slice, steps=1)
    if "error" in pred or not pred.get("predictions"):
        continue

    p = pred["predictions"][0]
    pred_close = p["predicted_close"]
    actual_close = float(next_day["Close"])
    actual_open = float(next_day["Open"])

    # Simple strategy: BUY if predicted up > 0.5%, SELL if predicted down > 0.5%
    pred_change = (pred_close - float(train_slice["Close"].iloc[-1])) / float(train_slice["Close"].iloc[-1]) * 100

    action = "HOLD"
    if pred_change > 0.5 and shares == 0:
        # BUY at open
        shares = int(cash / actual_open)
        cost = shares * actual_open
        cash -= cost
        action = f"BUY {shares}@${actual_open:.2f}"
    elif pred_change < -0.5 and shares > 0:
        # SELL at open
        revenue = shares * actual_open
        cash += revenue
        action = f"SELL {shares}@${actual_open:.2f}"
        shares = 0

    portfolio = cash + shares * actual_close
    trades.append({
        "date": next_day["Date"],
        "pred_close": pred_close,
        "actual_close": actual_close,
        "action": action,
        "portfolio": portfolio,
    })

print(f"  Initial Capital: ${initial_capital:,.2f}")
print(f"  Strategy: Buy if Kronos predicts >+0.5%, Sell if <-0.5%")
print()
print(f"  {'Date':<12} {'Action':<22} {'Pred Close':>11} {'Act Close':>10} {'Portfolio':>12}")
print(f"  {'-' * 70}")
for t in trades:
    print(f"  {t['date']:<12} {t['action']:<22} ${t['pred_close']:>10.2f} ${t['actual_close']:>9.2f} ${t['portfolio']:>11,.2f}")

final_portfolio = trades[-1]["portfolio"] if trades else initial_capital
pnl = final_portfolio - initial_capital
pnl_pct = (pnl / initial_capital) * 100

# Buy and hold comparison
bnh_start = float(df_trade.iloc[5]["Open"])
bnh_end = float(df_trade.iloc[-1]["Close"])
bnh_shares = int(initial_capital / bnh_start)
bnh_final = (initial_capital - bnh_shares * bnh_start) + bnh_shares * bnh_end
bnh_pnl = bnh_final - initial_capital
bnh_pct = (bnh_pnl / initial_capital) * 100

print(f"\n  {'─' * 50}")
print(f"  Kronos Strategy:   ${final_portfolio:>11,.2f}  ({pnl_pct:>+.2f}%)")
print(f"  Buy & Hold:        ${bnh_final:>11,.2f}  ({bnh_pct:>+.2f}%)")
print(f"  Alpha:             {pnl_pct - bnh_pct:>+.2f}%")

print(f"\n{'=' * 70}")
print("REPORT COMPLETE")
print(f"{'=' * 70}")
