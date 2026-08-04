import React, { useEffect, useRef, useState, useCallback } from 'react';
import { createChart } from 'lightweight-charts';
import axios from 'axios';
import PaperTradingPanel from './PaperTradingPanel';
import SimBanner from './SimBanner';

const API = process.env.REACT_APP_API_URL || 'http://127.0.0.1:8000/api';
const WS_URL = process.env.REACT_APP_WS_URL || 'ws://127.0.0.1:8000';

/** Deduplicate + sort array by time (numeric) */
function dedupNum(arr) {
  const map = new Map();
  for (const item of arr) map.set(item.time, item);
  return Array.from(map.values()).sort((a, b) => a.time - b.time);
}

/* ─────────────────────────────────────────────────────────────────
   LIVE 1-SECOND CHART  (left pane)
   WebSocket connects IMMEDIATELY — history loads in parallel
   ───────────────────────────────────────────────────────────────── */
function LiveSecondChart({ symbol, onTick }) {
  const containerRef = useRef(null);
  const chartRef = useRef(null);
  const wsRef = useRef(null);

  useEffect(() => {
    if (!containerRef.current) return;
    const container = containerRef.current;
    let disposed = false;

    const chart = createChart(container, {
      layout: { background: { color: '#0a0e17' }, textColor: '#94a3b8' },
      grid: { vertLines: { color: '#1e293b33' }, horzLines: { color: '#1e293b33' } },
      crosshair: { mode: 0 },
      rightPriceScale: { borderColor: '#1e293b' },
      timeScale: { borderColor: '#1e293b', timeVisible: true, secondsVisible: true },
      width: container.clientWidth,
      height: container.clientHeight || 500,
    });

    const candleSeries = chart.addCandlestickSeries({
      upColor: '#22c55e', downColor: '#ef4444',
      borderUpColor: '#22c55e', borderDownColor: '#ef4444',
      wickUpColor: '#22c55e', wickDownColor: '#ef4444',
    });
    const volSeries = chart.addHistogramSeries({
      color: '#3b82f6', priceFormat: { type: 'volume' }, priceScaleId: 'vol',
    });
    chart.priceScale('vol').applyOptions({ scaleMargins: { top: 0.87, bottom: 0 } });
    const smaLine = chart.addLineSeries({ color: '#eab308', lineWidth: 1 });

    chartRef.current = { chart, candleSeries, volSeries, smaLine, smaBuffer: [] };
    const candleMap = {};
    let historyLoaded = false;

    // ── PARALLEL: Load history (non-blocking) ───────────────────
    axios.get(`${API}/realtime/${symbol}/candles?count=120`).then(res => {
      if (disposed) return;
      const candles = res.data.candles || [];
      const seen = new Set();
      const data = candles.filter(d => d.Date && d.Open).map(d => ({
        time: Math.floor(new Date(d.Date + (d.Date.length <= 16 ? ':00Z' : 'Z')).getTime() / 1000),
        open: d.Open, high: d.High, low: d.Low, close: d.Close,
      })).filter(d => { if (seen.has(d.time)) return false; seen.add(d.time); return true; })
        .sort((a, b) => a.time - b.time);
      if (data.length > 0 && chartRef.current) {
        candleSeries.setData(data);
        chart.timeScale().fitContent();
        historyLoaded = true;
      }
    }).catch(() => {});

    // ── PARALLEL: Connect WebSocket immediately ─────────────────
    let reconnectTimer;
    const connect = () => {
      const ws = new WebSocket(`${WS_URL}/ws/live/${symbol}`);
      wsRef.current = ws;

      ws.onmessage = (evt) => {
        if (disposed) return;
        const tick = JSON.parse(evt.data);
        const price = tick.price;
        if (!price) return;

        const now = Math.floor(Date.now() / 1000);
        const obj = chartRef.current;
        if (!obj) return;

        if (!candleMap[now]) {
          candleMap[now] = { time: now, open: price, high: price, low: price, close: price };
        } else {
          candleMap[now].high = Math.max(candleMap[now].high, price);
          candleMap[now].low = Math.min(candleMap[now].low, price);
          candleMap[now].close = price;
        }

        try {
          obj.candleSeries.update(candleMap[now]);
          obj.volSeries.update({
            time: now, value: tick.last_candle?.volume || 0,
            color: candleMap[now].close >= candleMap[now].open
              ? 'rgba(34,197,94,0.2)' : 'rgba(239,68,68,0.2)',
          });
          obj.smaBuffer.push({ time: now, close: price });
          if (obj.smaBuffer.length > 20) {
            const avg = obj.smaBuffer.slice(-20).reduce((s, c) => s + c.close, 0) / 20;
            obj.smaLine.update({ time: now, value: avg });
          }
        } catch (e) {}

        onTick && onTick({
          price, time: now,
          change: tick.change, change_percent: tick.change_percent,
          high: tick.high, low: tick.low, volume: tick.volume,
        });
      };

      ws.onclose = () => { if (!disposed) reconnectTimer = setTimeout(connect, 500); };
      ws.onerror = () => ws.close();
    };
    connect();

    const ro = new ResizeObserver(() => {
      if (!disposed) requestAnimationFrame(() => {
        if (!disposed) chart.applyOptions({ width: container.clientWidth, height: container.clientHeight });
      });
    });
    ro.observe(container);

    return () => {
      disposed = true;
      ro.disconnect();
      if (wsRef.current) wsRef.current.close();
      if (reconnectTimer) clearTimeout(reconnectTimer);
      chartRef.current = null;
      try { chart.remove(); } catch (e) {}
    };
  }, [symbol]);

  return <div ref={containerRef} style={{ width: '100%', height: '100%', minHeight: 500 }} />;
}


/* ─────────────────────────────────────────────────────────────────
   PREDICTED SECOND-BY-SECOND CHART  (right pane)
   Instant first frame from WebSocket snapshot, then real-time
   ───────────────────────────────────────────────────────────────── */
function PredictedSecondChart({ symbol, onPredUpdate }) {
  const containerRef = useRef(null);
  const chartRef = useRef(null);
  const wsRef = useRef(null);

  useEffect(() => {
    if (!containerRef.current) return;
    const container = containerRef.current;
    let disposed = false;

    const chart = createChart(container, {
      layout: { background: { color: '#0a0e17' }, textColor: '#94a3b8' },
      grid: { vertLines: { color: '#1e293b33' }, horzLines: { color: '#1e293b33' } },
      crosshair: { mode: 0 },
      rightPriceScale: { borderColor: '#1e293b' },
      timeScale: { borderColor: '#1e293b', timeVisible: true, secondsVisible: true },
      width: container.clientWidth,
      height: container.clientHeight || 500,
    });

    const predLine = chart.addLineSeries({ color: '#a855f7', lineWidth: 2 });
    const kronosLine = chart.addLineSeries({ color: 'rgba(100,116,139,0.4)', lineWidth: 1, lineStyle: 3 });
    const priceLine = chart.addLineSeries({ color: '#22c55e', lineWidth: 1, lineStyle: 2 });
    const actualLine = chart.addLineSeries({ color: 'rgba(34,197,94,0.5)', lineWidth: 1 });

    chartRef.current = { chart, predLine, kronosLine, priceLine, actualLine, actualBuffer: [] };

    // ── Connect IMMEDIATELY — server sends instant snapshot ─────
    let reconnectTimer;
    const connect = () => {
      const ws = new WebSocket(`${WS_URL}/ws/predict/${symbol}`);
      wsRef.current = ws;

      ws.onmessage = (evt) => {
        if (disposed) return;
        const data = JSON.parse(evt.data);
        const obj = chartRef.current;
        if (!obj || !data.predictions) return;

        const now = Math.floor(Date.now() / 1000);
        const current = data.current_price;

        // Actual price trail (dedup same-second)
        const last = obj.actualBuffer[obj.actualBuffer.length - 1];
        if (!last || last.time !== now) {
          obj.actualBuffer.push({ time: now, value: current });
        } else {
          last.value = current;
        }
        if (obj.actualBuffer.length > 120) obj.actualBuffer.shift();

        // Use update() for trailing lines — faster than setData()
        try { actualLine.update({ time: now, value: current }); } catch (e) {}

        // Price reference line
        try {
          obj.priceLine.setData([
            { time: now, value: current },
            { time: now + 35, value: current },
          ]);
        } catch (e) {}

        // Predicted + Kronos lines
        const predData = [{ time: now, value: current }];
        const kronosData = [{ time: now, value: current }];
        for (const p of data.predictions) {
          predData.push({ time: now + p.seconds_ahead, value: p.predicted_price });
          kronosData.push({ time: now + p.seconds_ahead, value: p.kronos_price });
        }

        try {
          obj.predLine.setData(predData);
          obj.kronosLine.setData(kronosData);
          obj.chart.timeScale().fitContent();
        } catch (e) {}

        onPredUpdate && onPredUpdate(data);
      };

      ws.onclose = () => { if (!disposed) reconnectTimer = setTimeout(connect, 500); };
      ws.onerror = () => ws.close();
    };
    connect();

    const ro = new ResizeObserver(() => {
      if (!disposed) requestAnimationFrame(() => {
        if (!disposed) chart.applyOptions({ width: container.clientWidth, height: container.clientHeight });
      });
    });
    ro.observe(container);

    return () => {
      disposed = true;
      ro.disconnect();
      if (wsRef.current) wsRef.current.close();
      if (reconnectTimer) clearTimeout(reconnectTimer);
      chartRef.current = null;
      try { chart.remove(); } catch (e) {}
    };
  }, [symbol]);

  return <div ref={containerRef} style={{ width: '100%', height: '100%', minHeight: 500 }} />;
}


/* ─────────────────────────────────────────────────────────────────
   PATTERN SIGNAL BAR
   ───────────────────────────────────────────────────────────────── */
function PatternSignalBar({ pattern, atr, kronosAge }) {
  if (!pattern) return null;
  const { signal, direction, confidence, momentum, trend, reversion } = pattern;
  const color = direction === 'bullish' ? '#22c55e' : direction === 'bearish' ? '#ef4444' : '#eab308';

  const MiniBar = ({ val, label, maxVal = 1 }) => {
    const v = val ?? 0;
    const pct = Math.abs(v) / maxVal * 100;
    const c = v > 0.01 ? '#22c55e' : v < -0.01 ? '#ef4444' : '#475569';
    return (
      <div style={{ flex: 1, minWidth: 80 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.65rem', color: '#64748b', marginBottom: 1 }}>
          <span>{label}</span>
          <span style={{ color: c, fontWeight: 600 }}>{v > 0 ? '+' : ''}{v.toFixed(3)}</span>
        </div>
        <div style={{ height: 4, background: '#1e293b', borderRadius: 2, overflow: 'hidden', position: 'relative' }}>
          <div style={{ position: 'absolute', left: '50%', top: 0, bottom: 0, width: 1, background: '#334155' }} />
          <div style={{
            position: 'absolute',
            left: v >= 0 ? '50%' : `${50 - pct / 2}%`,
            width: `${pct / 2}%`,
            top: 0, bottom: 0, background: c, borderRadius: 2,
          }} />
        </div>
      </div>
    );
  };

  return (
    <div style={{
      display: 'flex', gap: 12, alignItems: 'center',
      padding: '8px 14px', borderRadius: 6,
      background: `${color}08`, border: `1px solid ${color}22`,
    }}>
      <div style={{ textAlign: 'center', minWidth: 70 }}>
        <div style={{ fontSize: '1.4rem', color }}>{direction === 'bullish' ? '▲' : direction === 'bearish' ? '▼' : '◆'}</div>
        <div style={{ fontSize: '0.85rem', fontWeight: 700, color }}>{(signal ?? 0) > 0 ? '+' : ''}{(signal ?? 0).toFixed(3)}</div>
        <div style={{ fontSize: '0.6rem', color: '#64748b' }}>Score</div>
      </div>
      <div style={{ flex: 1, display: 'flex', gap: 10 }}>
        <MiniBar val={momentum} label="Momentum" maxVal={5} />
        <MiniBar val={trend} label="Trend" maxVal={2} />
        <MiniBar val={reversion} label="Reversion" maxVal={1} />
      </div>
      <div style={{ textAlign: 'right', fontSize: '0.65rem', color: '#475569', minWidth: 90 }}>
        <div>ATR: {atr?.toFixed(3)}</div>
        <div>Conf: {(confidence * 100).toFixed(0)}%</div>
        <div>Kronos: {kronosAge?.toFixed(0)}s ago</div>
      </div>
    </div>
  );
}


/* ─────────────────────────────────────────────────────────────────
   PREDICTION CHIPS
   ───────────────────────────────────────────────────────────────── */
function PredictionChips({ predictions, currentPrice }) {
  if (!predictions || predictions.length === 0) return null;
  const keySeconds = [5, 10, 30, 60];
  const chips = keySeconds.map(s => predictions.find(p => p.seconds_ahead === s)).filter(Boolean);

  return (
    <div style={{ display: 'flex', gap: 4, marginTop: 6 }}>
      {chips.map((p, i) => (
        <div key={i} style={{
          flex: 1, textAlign: 'center', padding: '5px 3px',
          background: p.direction === 'bullish' ? 'rgba(34,197,94,0.06)' : 'rgba(239,68,68,0.06)',
          borderRadius: 6,
          border: `1px solid ${p.direction === 'bullish' ? 'rgba(34,197,94,0.12)' : 'rgba(239,68,68,0.12)'}`,
        }}>
          <div style={{ fontSize: '0.6rem', color: '#475569' }}>+{p.seconds_ahead}s</div>
          <div style={{ fontSize: '0.8rem', fontWeight: 700, color: p.direction === 'bullish' ? '#22c55e' : '#ef4444' }}>
            ${(p.predicted_price ?? 0).toFixed(2)}
          </div>
          <div style={{ fontSize: '0.65rem', fontWeight: 600, color: p.direction === 'bullish' ? '#22c55e' : '#ef4444' }}>
            {(p.change_percent ?? 0) > 0 ? '+' : ''}{(p.change_percent ?? 0).toFixed(3)}%
          </div>
        </div>
      ))}
    </div>
  );
}


/* ─────────────────────────────────────────────────────────────────
   MAIN — LIVE TRADING PAGE
   ───────────────────────────────────────────────────────────────── */
function LiveTrading({ symbol }) {
  const [liveTick, setLiveTick] = useState(null);
  const [predData, setPredData] = useState(null);
  const [connected, setConnected] = useState(false);

  const handleTick = useCallback((tick) => {
    setLiveTick(tick);
    setConnected(true);
  }, []);

  const handlePred = useCallback((data) => {
    setPredData(data);
  }, []);

  return (
    <div>
      {/* Status bar */}
      <div style={{
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        marginBottom: 8, padding: '8px 16px',
        background: '#111827', borderRadius: 8, border: '1px solid #1e293b',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span className="live-dot" style={{ width: 8, height: 8, background: connected ? '#22c55e' : '#ef4444' }} />
            <span style={{ fontSize: '1rem', fontWeight: 700, color: '#e2e8f0' }}>{symbol}</span>
          </div>
          {liveTick && (
            <span style={{ fontSize: '1.2rem', fontWeight: 700, color: (liveTick.change_percent || 0) >= 0 ? '#22c55e' : '#ef4444' }}>
              ${liveTick.price?.toFixed(2)}
              <span style={{ fontSize: '0.8rem', marginLeft: 6 }}>
                {(liveTick.change_percent || 0) >= 0 ? '+' : ''}{(liveTick.change_percent || 0).toFixed(2)}%
              </span>
            </span>
          )}
          {liveTick?.high && (
            <span style={{ fontSize: '0.75rem', color: '#475569' }}>
              H: ${liveTick.high?.toFixed(2)} &nbsp; L: ${liveTick.low?.toFixed(2)} &nbsp; Vol: {(liveTick.volume || 0).toLocaleString()}
            </span>
          )}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: '0.75rem', color: '#475569' }}>
          <span>{predData?.micro_candles || 0} micro-candles</span>
          <span style={{ padding: '2px 8px', borderRadius: 4, fontWeight: 600, background: 'rgba(168,85,247,0.1)', color: '#a855f7', fontSize: '0.7rem' }}>
            ZERO-DELAY STREAMING
          </span>
        </div>
      </div>

      <SimBanner what="The paper-trading P&L below"
                 note="This panel's job is showing what the signals decided, not what they earned." />

      {/* Pattern signal bar */}
      <PatternSignalBar pattern={predData?.pattern} atr={predData?.atr} kronosAge={predData?.kronos_age_seconds} />

      {/* Triple layout: Live Chart | Prediction Chart | Paper Trading */}
      <div className="live-trading-triple" style={{ marginTop: 8 }}>
        <div className="live-trading-pane">
          <div className="dual-chart-header">
            <span className="dual-chart-title">
              <span className="live-dot" style={{ width: 6, height: 6 }} /> LIVE — {symbol} (1-sec candles)
            </span>
            <div style={{ display: 'flex', gap: 12, fontSize: '0.7rem', color: '#64748b' }}>
              <span><span style={{ color: '#eab308' }}>—</span> SMA-20</span>
            </div>
          </div>
          <LiveSecondChart symbol={symbol} onTick={handleTick} />
        </div>

        <div className="live-trading-pane">
          <div className="dual-chart-header">
            <span className="dual-chart-title" style={{ color: '#a855f7' }}>
              KRONOS AI — Per-Second Prediction
            </span>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: '0.7rem' }}>
              {predData && (
                <>
                  <span style={{ color: '#64748b' }}><span style={{ color: '#a855f7' }}>—</span> Adjusted</span>
                  <span style={{ color: '#64748b' }}><span style={{ color: '#475569' }}>- -</span> Raw Kronos</span>
                  <span style={{ color: '#64748b' }}><span style={{ color: '#22c55e' }}>—</span> Actual</span>
                </>
              )}
            </div>
          </div>
          <PredictedSecondChart symbol={symbol} onPredUpdate={handlePred} />
          <PredictionChips predictions={predData?.predictions} currentPrice={predData?.current_price} />
          {predData && (
            <div style={{
              marginTop: 6, padding: '5px 10px', borderRadius: 4,
              background: 'rgba(168,85,247,0.06)', border: '1px solid rgba(168,85,247,0.12)',
              fontSize: '0.65rem', color: '#a855f7', display: 'flex', justifyContent: 'space-between',
            }}>
              <span>Kronos GPU (cuda:0) + Pattern Blend | {predData.micro_candles} buffer</span>
              <span>Kronos: {predData.kronos_age_seconds?.toFixed(0)}s ago | ATR: {predData.atr?.toFixed(4)}</span>
            </div>
          )}
        </div>

        <div className="live-trading-pane paper-pane">
          <PaperTradingPanel />
        </div>
      </div>
    </div>
  );
}

export default LiveTrading;
