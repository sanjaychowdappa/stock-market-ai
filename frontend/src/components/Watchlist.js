import React from 'react';

const DEFAULT_WATCHLIST = ['AAPL', 'MSFT', 'GOOGL', 'AMZN', 'TSLA', 'NVDA', 'META', 'SPY', 'QQQ', 'AMD'];

function Watchlist({ onSelect }) {
  return (
    <div className="card">
      <div className="card-title">Watchlist</div>
      {DEFAULT_WATCHLIST.map((sym) => (
        <div key={sym} className="watchlist-item" onClick={() => onSelect(sym)}>
          <span className="watchlist-symbol">{sym}</span>
          <span style={{ fontSize: '0.8rem', color: '#3b82f6' }}>Analyze</span>
        </div>
      ))}
    </div>
  );
}

export default Watchlist;
