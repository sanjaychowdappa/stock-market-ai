import React, { useEffect, useRef } from 'react';
import { createChart } from 'lightweight-charts';

const WS_URL = process.env.REACT_APP_WS_URL || 'ws://127.0.0.1:8000';

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

    const connect = () => {
      ws = new WebSocket(`${WS_URL}/ws/live/${symbol}`);

      ws.onmessage = (event) => {
        const tick = JSON.parse(event.data);
        if (!tick.last_candle || disposed) return;

        const now = Math.floor(Date.now() / 1000);
        const bucket = now - (now % 60); // 1-minute candles

        const candle = {
          time: bucket,
          open: tick.last_candle.open,
          high: tick.last_candle.high,
          low: tick.last_candle.low,
          close: tick.price || tick.last_candle.close,
        };

        try {
          candleSeries.update(candle);
          volumeSeries.update({
            time: bucket,
            value: tick.last_candle.volume || 0,
            color: candle.close >= candle.open ? 'rgba(34,197,94,0.25)' : 'rgba(239,68,68,0.25)',
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
