import React, { useEffect, useRef, useState } from 'react';
import { createChart } from 'lightweight-charts';
import axios from 'axios';

const WS_URL = process.env.REACT_APP_WS_URL || 'ws://localhost:8000';
const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

function parseTime(dateStr, isIntraday) {
  if (isIntraday && dateStr.includes('T')) {
    // Handle both "2026-05-22T15:30" and "2026-05-22T15:30:00" formats
    const d = dateStr.endsWith('Z') ? dateStr : dateStr + (dateStr.length <= 16 ? ':00Z' : 'Z');
    return Math.floor(new Date(d).getTime() / 1000);
  }
  return dateStr.split('T')[0].split(' ')[0];
}

/** Deduplicate and sort chart data by time — lightweight-charts requires strictly ascending unique times */
function dedup(arr) {
  if (!arr.length) return arr;
  const seen = new Map();
  for (const item of arr) {
    const existing = seen.get(item.time);
    if (!existing) {
      seen.set(item.time, item);
    } else {
      // merge: keep latest values (last-write-wins)
      seen.set(item.time, { ...existing, ...item });
    }
  }
  const out = Array.from(seen.values());
  out.sort((a, b) => typeof a.time === 'number' ? a.time - b.time : (a.time < b.time ? -1 : 1));
  return out;
}

/* ── Left pane: Live chart with real-time WebSocket updates ─────── */
function LiveChart({ data, patterns, symbol, interval }) {
  const containerRef = useRef(null);
  const chartObjRef = useRef(null);
  const wsRef = useRef(null);
  const dataRef = useRef([]);

  useEffect(() => {
    if (!data || data.length === 0 || !containerRef.current) return;

    const container = containerRef.current;
    let disposed = false;
    const isIntraday = ['1m', '5m', '15m', '1h'].includes(interval);

    const chart = createChart(container, {
      layout: { background: { color: '#0f1523' }, textColor: '#94a3b8' },
      grid: { vertLines: { color: '#1e293b' }, horzLines: { color: '#1e293b' } },
      crosshair: { mode: 0 },
      rightPriceScale: { borderColor: '#1e293b' },
      timeScale: { borderColor: '#1e293b', timeVisible: true, secondsVisible: false },
      width: container.clientWidth,
      height: 420,
    });

    const candleSeries = chart.addCandlestickSeries({
      upColor: '#22c55e', downColor: '#ef4444',
      borderUpColor: '#22c55e', borderDownColor: '#ef4444',
      wickUpColor: '#22c55e', wickDownColor: '#ef4444',
    });

    const chartData = dedup(data
      .filter(d => d.Date && d.Open && d.Close)
      .map(d => ({
        time: parseTime(d.Date, isIntraday),
        open: d.Open, high: d.High, low: d.Low, close: d.Close,
      })));

    dataRef.current = chartData;
    candleSeries.setData(chartData);

    const volumeSeries = chart.addHistogramSeries({
      color: '#3b82f6', priceFormat: { type: 'volume' }, priceScaleId: 'volume',
    });
    chart.priceScale('volume').applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });
    volumeSeries.setData(dedup(
      data.filter(d => d.Date && d.Volume).map(d => ({
        time: parseTime(d.Date, isIntraday),
        value: d.Volume,
        color: d.Close >= d.Open ? 'rgba(34,197,94,0.25)' : 'rgba(239,68,68,0.25)',
      }))
    ));

    if (data[0]?.SMA_20) {
      const s = chart.addLineSeries({ color: '#eab308', lineWidth: 1 });
      s.setData(dedup(data.filter(d => d.SMA_20).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.SMA_20 }))));
    }
    if (data[0]?.BB_Upper) {
      const u = chart.addLineSeries({ color: 'rgba(59,130,246,0.4)', lineWidth: 1, lineStyle: 2 });
      const l = chart.addLineSeries({ color: 'rgba(59,130,246,0.4)', lineWidth: 1, lineStyle: 2 });
      u.setData(dedup(data.filter(d => d.BB_Upper).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.BB_Upper }))));
      l.setData(dedup(data.filter(d => d.BB_Lower).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.BB_Lower }))));
    }

    if (patterns && patterns.length > 0) {
      const seen = new Set();
      const markers = patterns
        .filter(p => p.index < data.length && data[p.index]?.Date)
        .filter(p => { if (seen.has(p.index)) return false; seen.add(p.index); return true; })
        .slice(0, 20)
        .map(p => ({
          time: parseTime(data[p.index].Date, isIntraday),
          position: p.type === 'bullish' ? 'belowBar' : 'aboveBar',
          color: p.type === 'bullish' ? '#22c55e' : p.type === 'bearish' ? '#ef4444' : '#eab308',
          shape: p.type === 'bullish' ? 'arrowUp' : p.type === 'bearish' ? 'arrowDown' : 'circle',
          text: p.name,
        }))
        .sort((a, b) => typeof a.time === 'number' ? a.time - b.time : a.time.localeCompare(b.time));
      if (markers.length > 0) candleSeries.setMarkers(markers);
    }

    chart.timeScale().fitContent();
    chartObjRef.current = { chart, candleSeries, volumeSeries };

    const ro = new ResizeObserver(() => {
      if (!disposed) chart.applyOptions({ width: container.clientWidth });
    });
    ro.observe(container);

    return () => {
      disposed = true;
      chartObjRef.current = null;
      ro.disconnect();
      try { chart.remove(); } catch (e) { /* */ }
      while (container.firstChild) container.removeChild(container.firstChild);
    };
  }, [data, patterns, interval]);

  useEffect(() => {
    if (!symbol) return;
    const isIntraday = ['1m', '5m', '15m', '1h'].includes(interval);
    if (!isIntraday) return;

    let reconnectTimer;
    const connect = () => {
      const ws = new WebSocket(`${WS_URL}/ws/live/${symbol}`);
      wsRef.current = ws;

      ws.onmessage = (event) => {
        const tick = JSON.parse(event.data);
        const obj = chartObjRef.current;
        if (!obj || !tick.last_candle) return;

        const now = Math.floor(Date.now() / 1000);
        let bucket;
        if (interval === '1m') bucket = now - (now % 60);
        else if (interval === '5m') bucket = now - (now % 300);
        else if (interval === '15m') bucket = now - (now % 900);
        else bucket = now - (now % 3600);

        const candle = {
          time: bucket,
          open: tick.last_candle.open,
          high: tick.last_candle.high,
          low: tick.last_candle.low,
          close: tick.price,
        };

        try {
          obj.candleSeries.update(candle);
          obj.volumeSeries.update({
            time: bucket,
            value: tick.last_candle.volume,
            color: candle.close >= candle.open ? 'rgba(34,197,94,0.25)' : 'rgba(239,68,68,0.25)',
          });
        } catch (e) { /* chart may be disposed */ }
      };

      ws.onclose = () => { reconnectTimer = setTimeout(connect, 3000); };
      ws.onerror = () => ws.close();
    };

    connect();
    return () => {
      if (wsRef.current) wsRef.current.close();
      if (reconnectTimer) clearTimeout(reconnectTimer);
    };
  }, [symbol, interval]);

  return <div ref={containerRef} />;
}

/* ── Right pane: Kronos Foundation Model Prediction ───────────── */
function KronosPredictionChart({ symbol, prediction }) {
  const containerRef = useRef(null);
  const [kronosPred, setKronosPred] = useState(null);
  const [kronosLoading, setKronosLoading] = useState(false);
  const [kronosError, setKronosError] = useState(null);

  useEffect(() => {
    const fetchKronos = async () => {
      setKronosLoading(true);
      setKronosError(null);
      try {
        const res = await axios.get(`${API}/predictions/${symbol}/kronos?steps=7`);
        if (res.data.error) {
          setKronosError(res.data.error);
          setKronosPred(null);
        } else {
          setKronosPred(res.data);
        }
      } catch (e) {
        setKronosError('Failed to fetch Kronos predictions');
      }
      setKronosLoading(false);
    };
    fetchKronos();
    const timer = setInterval(fetchKronos, 60000); // refresh every 60s
    return () => clearInterval(timer);
  }, [symbol]);

  // Render chart when data arrives
  useEffect(() => {
    if (!kronosPred || !kronosPred.predictions || kronosPred.predictions.length === 0) return;
    if (!containerRef.current) return;

    const container = containerRef.current;
    let disposed = false;

    const chart = createChart(container, {
      layout: { background: { color: '#0f1523' }, textColor: '#94a3b8' },
      grid: { vertLines: { color: '#1e293b' }, horzLines: { color: '#1e293b' } },
      crosshair: { mode: 0 },
      rightPriceScale: { borderColor: '#1e293b' },
      timeScale: { borderColor: '#1e293b', timeVisible: false },
      width: container.clientWidth,
      height: 420,
    });

    const closeLine = [];

    // Build prediction candles only (no anchor that could duplicate first date)
    const predDates = new Set();
    const predCandles = [];

    for (const p of kronosPred.predictions) {
      if (predDates.has(p.date)) continue; // skip any duplicates from API
      predDates.add(p.date);
      predCandles.push({
        time: p.date,
        open: p.predicted_open,
        high: p.predicted_high,
        low: p.predicted_low,
        close: p.predicted_close,
      });
      closeLine.push({ time: p.date, value: p.predicted_close });
    }

    // Add current-price anchor ONLY if its date doesn't collide with any prediction
    const todayStr = new Date().toISOString().split('T')[0];
    const anchorDate = predDates.has(todayStr)
      ? null  // skip anchor, first pred candle already covers today
      : todayStr;

    if (anchorDate) {
      predCandles.unshift({
        time: anchorDate,
        open: kronosPred.current_close,
        high: kronosPred.current_close,
        low: kronosPred.current_close,
        close: kronosPred.current_close,
      });
      closeLine.unshift({ time: anchorDate, value: kronosPred.current_close });
    }

    const candleSeries = chart.addCandlestickSeries({
      upColor: 'rgba(168,85,247,0.6)', downColor: 'rgba(239,68,68,0.6)',
      borderUpColor: '#a855f7', borderDownColor: '#ef4444',
      wickUpColor: '#a855f7', wickDownColor: '#ef4444',
    });
    candleSeries.setData(predCandles);

    const trendLine = chart.addLineSeries({
      color: '#a855f7', lineWidth: 2, lineStyle: 2,
    });
    trendLine.setData(closeLine);

    const markerDate = anchorDate || kronosPred.predictions[0]?.date;
    candleSeries.setMarkers([{
      time: markerDate,
      position: 'belowBar',
      color: '#a855f7',
      shape: 'circle',
      text: 'NOW',
    }]);

    chart.timeScale().fitContent();

    const ro = new ResizeObserver(() => {
      if (!disposed) chart.applyOptions({ width: container.clientWidth });
    });
    ro.observe(container);

    return () => {
      disposed = true;
      ro.disconnect();
      try { chart.remove(); } catch (e) { /* */ }
      while (container.firstChild) container.removeChild(container.firstChild);
    };
  }, [kronosPred]);

  const direction = kronosPred ? kronosPred.direction : 'neutral';
  const totalChange = kronosPred ? kronosPred.total_change_percent : 0;

  return (
    <div>
      {/* Summary bar */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          {kronosLoading ? (
            <span style={{ fontSize: '0.85rem', color: '#a855f7' }}>Loading Kronos model...</span>
          ) : kronosError ? (
            <span style={{ fontSize: '0.85rem', color: '#ef4444' }}>{kronosError}</span>
          ) : kronosPred ? (
            <>
              <span style={{
                fontSize: '1.5rem',
                color: direction === 'bullish' ? '#22c55e' : '#ef4444',
              }}>
                {direction === 'bullish' ? '▲' : '▼'}
              </span>
              <span style={{
                fontSize: '1.1rem', fontWeight: 700,
                color: direction === 'bullish' ? '#22c55e' : '#ef4444',
              }}>
                {totalChange > 0 ? '+' : ''}{totalChange}%
              </span>
            </>
          ) : null}
        </div>
        {kronosPred && (
          <div style={{ display: 'flex', gap: 12, fontSize: '0.75rem', color: '#64748b' }}>
            <span>Now: ${kronosPred.current_close}</span>
            <span style={{ color: direction === 'bullish' ? '#22c55e' : '#ef4444', fontWeight: 600 }}>
              Target: ${kronosPred.final_predicted_close}
            </span>
          </div>
        )}
      </div>

      {/* Chart */}
      <div ref={containerRef} />

      {/* Per-day change chips */}
      {kronosPred && kronosPred.predictions && kronosPred.predictions.length > 0 && (
        <div style={{ display: 'flex', gap: 4, marginTop: 6 }}>
          {kronosPred.predictions.map((p, i) => (
            <div key={i} style={{
              flex: 1, textAlign: 'center', padding: '4px 2px',
              background: p.direction === 'bullish' ? 'rgba(34,197,94,0.1)' : 'rgba(239,68,68,0.1)',
              borderRadius: 4, fontSize: '0.7rem',
            }}>
              <div style={{ color: '#64748b' }}>D{i + 1}</div>
              <div style={{
                fontWeight: 700, fontSize: '0.75rem',
                color: p.direction === 'bullish' ? '#22c55e' : '#ef4444',
              }}>
                {p.change_percent > 0 ? '+' : ''}{p.change_percent}%
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Model info badge */}
      {kronosPred && (
        <div style={{
          marginTop: 8, padding: '6px 10px', borderRadius: 4,
          background: 'rgba(168,85,247,0.08)', border: '1px solid rgba(168,85,247,0.2)',
          fontSize: '0.7rem', color: '#a855f7', display: 'flex', justifyContent: 'space-between',
        }}>
          <span>{kronosPred.model}</span>
          <span>T={kronosPred.parameters?.temperature} | top_p={kronosPred.parameters?.top_p} | samples={kronosPred.parameters?.sample_count}</span>
        </div>
      )}
    </div>
  );
}

/* ── Main export: side-by-side layout ──────────────────────────── */
function CandlestickChart({ data, patterns, prediction, symbol, interval }) {
  return (
    <div>
      <div className="dual-chart-grid">
        <div className="dual-chart-pane">
          <div className="dual-chart-header">
            <span className="dual-chart-title">
              <span className="live-dot" style={{ width: 6, height: 6 }} /> LIVE — {symbol}
            </span>
            <div style={{ display: 'flex', gap: 12, fontSize: '0.7rem', color: '#64748b' }}>
              <span><span style={{ color: '#eab308' }}>—</span> SMA</span>
              <span><span style={{ color: '#3b82f6', opacity: 0.5 }}>- -</span> BB</span>
            </div>
          </div>
          <LiveChart data={data} patterns={patterns} symbol={symbol} interval={interval} />
        </div>
        <div className="dual-chart-pane">
          <div className="dual-chart-header">
            <span className="dual-chart-title" style={{ color: '#a855f7' }}>
              KRONOS AI — 7 Day Forecast
            </span>
            <span style={{ fontSize: '0.7rem', color: '#64748b' }}>
              Foundation Model — 12B K-lines pre-trained
            </span>
          </div>
          <KronosPredictionChart symbol={symbol} prediction={prediction} />
        </div>
      </div>
    </div>
  );
}

export default CandlestickChart;
