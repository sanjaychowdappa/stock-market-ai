import React from 'react';

function SignalPanel({ signals }) {
  if (!signals) return <div className="card"><p className="text-muted">Loading signals...</p></div>;

  const signalClass = signals.signal === 'BUY' ? 'signal-buy' : signals.signal === 'SELL' ? 'signal-sell' : 'signal-hold';

  return (
    <div className="card">
      <div className="card-title">Trading Signals</div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
        <span className={`signal-badge ${signalClass}`}>{signals.signal}</span>
        <span style={{ fontSize: '0.85rem', color: '#94a3b8' }}>
          Confidence: {(signals.confidence * 100).toFixed(0)}%
        </span>
      </div>

      <div style={{ marginBottom: 12 }}>
        <div className="stat-row">
          <span className="stat-label">Price</span>
          <span className="stat-value">${signals.current_price}</span>
        </div>
        <div className="stat-row">
          <span className="stat-label">Score</span>
          <span className={`stat-value ${signals.score > 0 ? 'positive' : signals.score < 0 ? 'negative' : ''}`}>
            {signals.score > 0 ? '+' : ''}{signals.score}
          </span>
        </div>
        {signals.stop_loss?.recommended && (
          <div className="stat-row">
            <span className="stat-label">Stop Loss</span>
            <span className="stat-value negative">${signals.stop_loss.recommended}</span>
          </div>
        )}
      </div>

      {signals.reasons?.length > 0 && (
        <div>
          <div style={{ fontSize: '0.75rem', color: '#64748b', marginBottom: 4 }}>KEY REASONS</div>
          <ul className="reason-list">
            {signals.reasons.filter(r => r).map((reason, i) => (
              <li key={i}>{reason}</li>
            ))}
          </ul>
        </div>
      )}

      {signals.take_profit?.targets?.length > 0 && (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontSize: '0.75rem', color: '#64748b', marginBottom: 4 }}>TAKE PROFIT TARGETS</div>
          {signals.take_profit.targets.map((tp, i) => (
            <div key={i} className="stat-row">
              <span className="stat-label">TP {tp.level} ({tp.risk_reward})</span>
              <span className="stat-value positive">${tp.price}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default SignalPanel;
