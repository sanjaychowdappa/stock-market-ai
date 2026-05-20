import React, { useEffect, useRef, useState } from 'react';
import { createChart } from 'lightweight-charts';
import axios from 'axios';

const WS_URL = process.env.REACT_APP_WS_URL || 'ws://localhost:8000';
const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

function parseTime(dateStr, isIntraday) {
  if (isIntraday && dateStr.includes('T')) {
    return Math.floor(new Date(dateStr + ':00Z').getTime() / 1000);
  }
  return dateStr.split('T')[0].split(' ')[0];
}

/* ── Left pane: Live chart with real-time WebSocket updates ─────── */
function LiveChart({ data, patterns, symbol, interval }) {
  const containerRef = useRef(null);
  const chartObjRef = useRef(null);     // { chart, candleSeries, volumeSeries, lineSeries }
  const wsRef = useRef(null);
  const dataRef = useRef([]);

  // Build chart once
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

    const chartData = data
      .filter(d => d.Date && d.Open && d.Close)
      .map(d => ({
        time: parseTime(d.Date, isIntraday),
        open: d.Open, high: d.High, low: d.Low, close: d.Close,
      }));

    dataRef.current = chartData;
    candleSeries.setData(chartData);

    const volumeSeries = chart.addHistogramSeries({
      color: '#3b82f6', priceFormat: { type: 'volume' }, priceScaleId: 'volume',
    });
    chart.priceScale('volume').applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });
    volumeSeries.setData(
      data.filter(d => d.Date && d.Volume).map(d => ({
        time: parseTime(d.Date, isIntraday),
        value: d.Volume,
        color: d.Close >= d.Open ? 'rgba(34,197,94,0.25)' : 'rgba(239,68,68,0.25)',
      }))
    );

    // Overlay lines
    if (data[0]?.SMA_20) {
      const s = chart.addLineSeries({ color: '#eab308', lineWidth: 1 });
      s.setData(data.filter(d => d.SMA_20).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.SMA_20 })));
    }
    if (data[0]?.BB_Upper) {
      const u = chart.addLineSeries({ color: 'rgba(59,130,246,0.4)', lineWidth: 1, lineStyle: 2 });
      const l = chart.addLineSeries({ color: 'rgba(59,130,246,0.4)', lineWidth: 1, lineStyle: 2 });
      u.setData(data.filter(d => d.BB_Upper).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.BB_Upper })));
      l.setData(data.filter(d => d.BB_Lower).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.BB_Lower })));
    }

    // Pattern markers
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

  // WebSocket: push live ticks into the chart every second
  useEffect(() => {
    if (!symbol) return;
    const isIntraday = ['1m', '5m', '15m', '1h'].includes(interval);
    if (!isIntraday) return; // only live-update intraday charts

    let reconnectTimer;
    const connect = () => {
      const ws = new WebSocket(`${WS_URL}/ws/live/${symbol}`);
      wsRef.current = ws;

      ws.onmessage = (event) => {
        const tick = JSON.parse(event.data);
        const obj = chartObjRef.current;
        if (!obj || !tick.last_candle) return;

        const now = Math.floor(Date.now() / 1000);
        // Round to current interval bucket
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

/* ── Right pane: Predicted chart with multi-step forecast ───────── */
function PredictionChart({ symbol, prediction }) {
  const containerRef = useRef(null);
  const [multiPred, setMultiPred] = useState(null);

  useEffect(() => {
    const fetch = async () => {
      try {
        const res = await axios.get(`${API}/predictions/${symbol}/multi?steps=7`);
        setMultiPred(res.data.predictions || []);
      } catch (e) { /* */ }
    };
    fetch();
    const timer = setInterval(fetch, 30000);
    return () => clearInterval(timer);
  }, [symbol]);

  useEffect(() => {
    if (!multiPred || multiPred.length === 0 || !containerRef.current) return;
    if (!prediction || prediction.error) return;

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

    // Build predicted candles starting from tomorrow
    const today = new Date();
    const predCandles = [];
    const closeLine = [];
    let d = new Date(today);

    // Add current price as starting point
    predCandles.push({
      time: today.toISOString().split('T')[0],
      open: prediction.current_close,
      high: prediction.current_close,
      low: prediction.current_close,
      close: prediction.current_close,
    });
    closeLine.push({
      time: today.toISOString().split('T')[0],
      value: prediction.current_close,
    });

    for (const p of multiPred) {
      d.setDate(d.getDate() + 1);
      while (d.getDay() === 0 || d.getDay() === 6) d.setDate(d.getDate() + 1);
      const dateStr = d.toISOString().split('T')[0];
      predCandles.push({
        time: dateStr,
        open: p.predicted_open,
        high: p.predicted_high,
        low: p.predicted_low,
        close: p.predicted_close,
      });
      closeLine.push({ time: dateStr, value: p.predicted_close });
    }

    const candleSeries = chart.addCandlestickSeries({
      upColor: 'rgba(34,197,94,0.6)', downColor: 'rgba(239,68,68,0.6)',
      borderUpColor: '#22c55e', borderDownColor: '#ef4444',
      wickUpColor: '#22c55e', wickDownColor: '#ef4444',
    });
    candleSeries.setData(predCandles);

    // Trend line overlay
    const trendLine = chart.addLineSeries({
      color: '#3b82f6', lineWidth: 2, lineStyle: 2,
    });
    trendLine.setData(closeLine);

    // Mark today
    candleSeries.setMarkers([{
      time: today.toISOString().split('T')[0],
      position: 'belowBar',
      color: '#3b82f6',
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
  }, [multiPred, prediction, symbol]);

  const pctChange = prediction && !prediction.error
    ? prediction.change_percent : 0;
  const direction = prediction && !prediction.error
    ? prediction.direction : 'neutral';

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
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
            {pctChange > 0 ? '+' : ''}{pctChange}%
          </span>
        </div>
        {prediction && !prediction.error && (
          <div style={{ display: 'flex', gap: 12, fontSize: '0.75rem', color: '#64748b' }}>
            <span>O: ${prediction.predicted_open}</span>
            <span style={{ color: '#22c55e' }}>H: ${prediction.predicted_high}</span>
            <span style={{ color: '#ef4444' }}>L: ${prediction.predicted_low}</span>
            <span style={{ fontWeight: 600, color: direction === 'bullish' ? '#22c55e' : '#ef4444' }}>
              C: ${prediction.predicted_close}
            </span>
          </div>
        )}
      </div>
      <div ref={containerRef} />
      {multiPred && multiPred.length > 0 && (
        <div style={{ display: 'flex', gap: 4, marginTop: 6 }}>
          {multiPred.map((p, i) => (
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
              PREDICTED — 7 Day Forecast
            </span>
            <span style={{ fontSize: '0.7rem', color: '#64748b' }}>
              XGBoost + GradientBoost + RandomForest
            </span>
          </div>
          <PredictionChart symbol={symbol} prediction={prediction} />
        </div>
      </div>
    </div>
  );
}

export default CandlestickChart;
