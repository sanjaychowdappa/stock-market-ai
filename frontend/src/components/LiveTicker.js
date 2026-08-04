import React, { useEffect, useState, useRef } from 'react';

const WS_URL = process.env.REACT_APP_WS_URL || 'ws://127.0.0.1:8000';

function LiveTicker({ symbol }) {
  const [data, setData] = useState(null);
  const [connected, setConnected] = useState(false);
  const [flash, setFlash] = useState(null);
  const prevPrice = useRef(null);
  const wsRef = useRef(null);
  const reconnectTimer = useRef(null);

  useEffect(() => {
    if (!symbol) return;

    const connect = () => {
      if (wsRef.current) {
        wsRef.current.close();
      }

      const ws = new WebSocket(`${WS_URL}/ws/live/${symbol}`);
      wsRef.current = ws;

      ws.onopen = () => setConnected(true);

      ws.onmessage = (event) => {
        const tick = JSON.parse(event.data);
        setData(tick);

        if (prevPrice.current !== null && tick.price !== prevPrice.current) {
          setFlash(tick.price > prevPrice.current ? 'up' : 'down');
          setTimeout(() => setFlash(null), 500);
        }
        prevPrice.current = tick.price;
      };

      ws.onclose = () => {
        setConnected(false);
        reconnectTimer.current = setTimeout(connect, 3000);
      };

      ws.onerror = () => ws.close();
    };

    connect();

    return () => {
      if (wsRef.current) wsRef.current.close();
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
    };
  }, [symbol]);

  if (!data) {
    return (
      <div className="live-ticker">
        <div className="live-dot" style={{ background: '#64748b' }} />
        <span className="live-symbol">{symbol}</span>
        <span style={{ color: '#64748b', fontSize: '0.85rem' }}>Connecting...</span>
      </div>
    );
  }

  const isUp = data.change >= 0;

  return (
    <div className={`live-ticker ${flash === 'up' ? 'flash-green' : flash === 'down' ? 'flash-red' : ''}`}>
      <div className="live-dot" style={{ background: connected ? '#22c55e' : '#ef4444' }} />
      <span className="live-symbol">{data.symbol}</span>
      <span className={`live-price ${flash === 'up' ? 'price-up' : flash === 'down' ? 'price-down' : ''}`}>
        ${data.price}
      </span>
      <span className={`live-change ${isUp ? 'positive' : 'negative'}`}>
        {isUp ? '+' : ''}{data.change} ({isUp ? '+' : ''}{data.change_percent}%)
      </span>
      <div className="live-stats">
        <span>H: ${data.high}</span>
        <span>L: ${data.low}</span>
        <span>Vol: {(data.volume / 1e6).toFixed(1)}M</span>
      </div>
    </div>
  );
}

export default LiveTicker;
