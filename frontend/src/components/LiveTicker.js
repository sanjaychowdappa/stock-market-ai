import React from 'react';

function LiveTicker({ data }) {
  if (!data || data.error) return null;

  const isUp = data.change >= 0;

  return (
    <div className="live-ticker">
      <div className="live-dot" />
      <span className="live-symbol">{data.symbol}</span>
      <span className="live-price">${data.price}</span>
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
