import React, { useEffect, useRef } from 'react';
import { createChart } from 'lightweight-charts';

function parseTime(dateStr, isIntraday) {
  if (isIntraday && dateStr.includes('T')) {
    return Math.floor(new Date(dateStr + ':00Z').getTime() / 1000);
  }
  return dateStr.split('T')[0].split(' ')[0];
}

function CandlestickChart({ data, patterns, prediction, symbol, interval }) {
  const containerRef = useRef(null);

  useEffect(() => {
    if (!data || data.length === 0 || !containerRef.current) return;

    const container = containerRef.current;
    let disposed = false;
    const isIntraday = ['1m', '5m', '15m', '1h'].includes(interval);

    const chart = createChart(container, {
      layout: {
        background: { color: '#1a2035' },
        textColor: '#94a3b8',
      },
      grid: {
        vertLines: { color: '#1e293b' },
        horzLines: { color: '#1e293b' },
      },
      crosshair: { mode: 0 },
      rightPriceScale: { borderColor: '#1e293b' },
      timeScale: {
        borderColor: '#1e293b',
        timeVisible: true,
        secondsVisible: false,
      },
      width: container.clientWidth,
      height: 460,
    });

    const chartData = data
      .filter(d => d.Date && d.Open && d.Close)
      .map(d => ({
        time: parseTime(d.Date, isIntraday),
        open: d.Open,
        high: d.High,
        low: d.Low,
        close: d.Close,
      }));

    if (chartData.length === 0) {
      return () => { if (!disposed) { disposed = true; chart.remove(); } };
    }

    const candleSeries = chart.addCandlestickSeries({
      upColor: '#22c55e',
      downColor: '#ef4444',
      borderDownColor: '#ef4444',
      borderUpColor: '#22c55e',
      wickDownColor: '#ef4444',
      wickUpColor: '#22c55e',
    });
    candleSeries.setData(chartData);

    const volumeSeries = chart.addHistogramSeries({
      color: '#3b82f6',
      priceFormat: { type: 'volume' },
      priceScaleId: 'volume',
    });
    chart.priceScale('volume').applyOptions({
      scaleMargins: { top: 0.8, bottom: 0 },
    });
    volumeSeries.setData(
      data
        .filter(d => d.Date && d.Volume)
        .map(d => ({
          time: parseTime(d.Date, isIntraday),
          value: d.Volume,
          color: d.Close >= d.Open ? 'rgba(34,197,94,0.3)' : 'rgba(239,68,68,0.3)',
        }))
    );

    if (data[0]?.SMA_20) {
      const sma20 = chart.addLineSeries({ color: '#eab308', lineWidth: 1, title: 'SMA 20' });
      sma20.setData(
        data.filter(d => d.SMA_20).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.SMA_20 }))
      );
    }

    if (data[0]?.SMA_50 && data[0].SMA_50 !== 0) {
      const sma50 = chart.addLineSeries({ color: '#a855f7', lineWidth: 1, title: 'SMA 50' });
      sma50.setData(
        data.filter(d => d.SMA_50 && d.SMA_50 !== 0).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.SMA_50 }))
      );
    }

    if (data[0]?.BB_Upper) {
      const bbUp = chart.addLineSeries({ color: 'rgba(59,130,246,0.4)', lineWidth: 1, lineStyle: 2 });
      const bbLow = chart.addLineSeries({ color: 'rgba(59,130,246,0.4)', lineWidth: 1, lineStyle: 2 });
      bbUp.setData(data.filter(d => d.BB_Upper).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.BB_Upper })));
      bbLow.setData(data.filter(d => d.BB_Lower).map(d => ({ time: parseTime(d.Date, isIntraday), value: d.BB_Lower })));
    }

    if (prediction && !prediction.error && chartData.length > 0 && !isIntraday) {
      const lastDate = chartData[chartData.length - 1].time;
      const nextDate = getNextBusinessDay(lastDate);
      const predSeries = chart.addCandlestickSeries({
        upColor: 'rgba(34,197,94,0.4)',
        downColor: 'rgba(239,68,68,0.4)',
        borderDownColor: 'rgba(239,68,68,0.6)',
        borderUpColor: 'rgba(34,197,94,0.6)',
        wickDownColor: 'rgba(239,68,68,0.6)',
        wickUpColor: 'rgba(34,197,94,0.6)',
      });
      predSeries.setData([{
        time: nextDate,
        open: prediction.predicted_open,
        high: prediction.predicted_high,
        low: prediction.predicted_low,
        close: prediction.predicted_close,
      }]);
    }

    if (patterns && patterns.length > 0) {
      const seen = new Set();
      const markers = patterns
        .filter(p => p.index < data.length && data[p.index]?.Date)
        .filter(p => {
          if (seen.has(p.index)) return false;
          seen.add(p.index);
          return true;
        })
        .slice(0, 20)
        .map(p => ({
          time: parseTime(data[p.index].Date, isIntraday),
          position: p.type === 'bullish' ? 'belowBar' : 'aboveBar',
          color: p.type === 'bullish' ? '#22c55e' : p.type === 'bearish' ? '#ef4444' : '#eab308',
          shape: p.type === 'bullish' ? 'arrowUp' : p.type === 'bearish' ? 'arrowDown' : 'circle',
          text: p.name,
        }))
        .sort((a, b) => {
          if (typeof a.time === 'number') return a.time - b.time;
          return a.time.localeCompare(b.time);
        });

      if (markers.length > 0) {
        candleSeries.setMarkers(markers);
      }
    }

    chart.timeScale().fitContent();

    const ro = new ResizeObserver(() => {
      if (!disposed) {
        chart.applyOptions({ width: container.clientWidth });
      }
    });
    ro.observe(container);

    return () => {
      disposed = true;
      ro.disconnect();
      try { chart.remove(); } catch (e) { /* strict mode */ }
      while (container.firstChild) {
        container.removeChild(container.firstChild);
      }
    };
  }, [data, patterns, prediction, interval]);

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
        <h2 style={{ fontSize: '1.1rem', fontWeight: 600 }}>{symbol}</h2>
        <div style={{ display: 'flex', gap: 16, fontSize: '0.75rem', color: '#94a3b8' }}>
          <span><span style={{ color: '#eab308' }}>---</span> SMA 20</span>
          <span><span style={{ color: '#a855f7' }}>---</span> SMA 50</span>
          <span><span style={{ color: '#3b82f6', opacity: 0.5 }}>- -</span> Bollinger</span>
          <span style={{ opacity: 0.5 }}>Ghost candle = prediction</span>
        </div>
      </div>
      <div ref={containerRef} />
    </div>
  );
}

function getNextBusinessDay(dateStr) {
  const parts = dateStr.split('-');
  const d = new Date(parseInt(parts[0]), parseInt(parts[1]) - 1, parseInt(parts[2]));
  d.setDate(d.getDate() + 1);
  while (d.getDay() === 0 || d.getDay() === 6) {
    d.setDate(d.getDate() + 1);
  }
  return d.toISOString().split('T')[0];
}

export default CandlestickChart;
