import React, { useEffect, useRef } from 'react';
import { createChart } from 'lightweight-charts';

const WS_URL = process.env.REACT_APP_WS_URL || 'ws://127.0.0.1:8000';
const API_URL = process.env.REACT_APP_API_URL || 'http://127.0.0.1:8000/api';

function CandlestickChart({ symbol }) {
  const containerRef = useRef(null);

  useEffect(() => {
    if (!symbol || !containerRef.current) return;

    const container = containerRef.current;
    let disposed = false;

    const chart = createChart(container, {
      layout: { background: { color: '#0f1523' }, textColor: '#94a3b8' },
      grid: { vertLines: { color: '#1e293b' }, horzLines: { color: '#1e293b' } },
      crosshair: { mode: 0 },
      rightPriceScale: { borderColor: '#1e293b' },
      timeScale: { borderColor: '#1e293b', timeVisible: true, secondsVisible: false },
      width: container.clientWidth,
      height: 500,
    });

    const candleSeries = chart.addCandlestickSeries({
      upColor: '#22c55e', downColor: '#ef4444',
      borderUpColor: '#22c55e', borderDownColor: '#ef4444',
      wickUpColor: '#22c55e', wickDownColor: '#ef4444',
    });

    const volumeSeries = chart.addHistogramSeries({
      color: '#3b82f6', priceFormat: { type: 'volume' }, priceScaleId: 'volume',
    });
    chart.priceScale('volume').applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });

    // Connect to live WebSocket for real-time candles
    let ws;
    let reconnectTimer;
    // The 1-minute bar currently being built from incoming trade ticks.
    let bar = null;

    // Seed with history before the socket takes over. Without this the chart
    // opens empty and draws forward one bar per minute, so a fresh load shows
    // nothing and a reload throws away everything already on screen.
    //
    // Ticks that arrive during the fetch are NOT dropped: they build `bar`
    // through the socket handler, and setData below is skipped once a live bar
    // exists, because setData would wipe it.
    fetch(`${API_URL}/bars/${symbol}`)
      .then((r) => r.json())
      .then((d) => {
        if (disposed || !Array.isArray(d.bars) || d.bars.length === 0) return;
        // Guard against the live handler having already started a bar: seeding
        // after that point would erase it and reset the volume accumulator.
        if (bar) return;
        candleSeries.setData(d.bars.map((b) => ({
          time: b.time, open: b.open, high: b.high, low: b.low, close: b.close,
        })));
        volumeSeries.setData(d.bars.map((b) => ({
          time: b.time,
          value: b.volume,
          color: b.close >= b.open ? 'rgba(34,197,94,0.25)' : 'rgba(239,68,68,0.25)',
        })));
        chart.timeScale().fitContent();
      })
      .catch(() => { /* live ticks still draw the chart forward */ });

    const connect = () => {
      ws = new WebSocket(`${WS_URL}/ws/live/${symbol}`);

      ws.onmessage = (event) => {
        if (disposed) return;
        let tick;
        try { tick = JSON.parse(event.data); } catch (e) { return; }

        // The stream sends trade ticks, NOT pre-built candles. This used to
        // read tick.last_candle and bail when it was missing — which is every
        // message — so the chart connected, received ticks, and drew nothing.
        // It was not slow to fill; it could never fill.
        const price = Number(tick.price);
        if (!Number.isFinite(price)) return;

        // Bucket on the tick's OWN timestamp. The old code used Date.now(),
        // which labels each bar with the browser's clock instead of the
        // exchange's and skews every candle by the client's drift.
        const ts = Number(tick.timestamp);
        const secs = Number.isFinite(ts) ? Math.floor(ts) : Math.floor(Date.now() / 1000);
        const bucket = secs - (secs % 60);

        if (!bar || bucket > bar.time) {
          bar = { time: bucket, open: price, high: price, low: price, close: price, volume: 0 };
        } else if (bucket < bar.time) {
          return; // late tick from a bar already closed — dropping beats rewriting
        } else {
          // Build the range from traded prices. tick.high/tick.low are NOT the
          // bar's range: the backend sends price +/- ATR (503.40 +/- 4.06 =
          // 507.46 / 499.34), so using them would draw wicks several dollars
          // wide on every candle.
          if (price > bar.high) bar.high = price;
          if (price < bar.low) bar.low = price;
          bar.close = price;
        }
        // `size` is this trade's quantity; `volume` is the cumulative session
        // total, so only `size` can accumulate into a per-bar figure.
        bar.volume += Number(tick.size) || 0;

        try {
          candleSeries.update({
            time: bar.time, open: bar.open, high: bar.high, low: bar.low, close: bar.close,
          });
          volumeSeries.update({
            time: bar.time,
            value: bar.volume,
            color: bar.close >= bar.open ? 'rgba(34,197,94,0.25)' : 'rgba(239,68,68,0.25)',
          });
        } catch (e) { /* chart may be disposed */ }
      };

      ws.onclose = () => {
        if (!disposed) reconnectTimer = setTimeout(connect, 3000);
      };
      ws.onerror = () => ws.close();
    };

    connect();

    const ro = new ResizeObserver(() => {
      if (!disposed) requestAnimationFrame(() => {
        if (!disposed) chart.applyOptions({ width: container.clientWidth });
      });
    });
    ro.observe(container);

    return () => {
      disposed = true;
      ro.disconnect();
      if (ws) ws.close();
      if (reconnectTimer) clearTimeout(reconnectTimer);
      try { chart.remove(); } catch (e) { /* */ }
      while (container.firstChild) container.removeChild(container.firstChild);
    };
  }, [symbol]);

  return (
    <div>
      <div style={{ marginBottom: 8, display: 'flex', alignItems: 'center', gap: 8 }}>
        <span className="live-dot" style={{ width: 6, height: 6 }} />
        <span style={{ fontWeight: 700 }}>LIVE — {symbol}</span>
        <span style={{ fontSize: '0.75rem', color: '#64748b' }}>1-minute candles from Alpaca WebSocket</span>
      </div>
      <div ref={containerRef} />
    </div>
  );
}

export default CandlestickChart;
