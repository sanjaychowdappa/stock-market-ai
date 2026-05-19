import React from 'react';

function PatternPanel({ patterns, symbol }) {
  if (!patterns) return <div className="card"><p>Loading patterns...</p></div>;

  return (
    <div>
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">Candlestick Pattern Analysis - {symbol}</div>
        <div style={{ display: 'flex', gap: 24, marginTop: 8 }}>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: '1.8rem', fontWeight: 700, color: '#22c55e' }}>{patterns.summary?.bullish || 0}</div>
            <div style={{ fontSize: '0.75rem', color: '#94a3b8' }}>Bullish</div>
          </div>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: '1.8rem', fontWeight: 700, color: '#ef4444' }}>{patterns.summary?.bearish || 0}</div>
            <div style={{ fontSize: '0.75rem', color: '#94a3b8' }}>Bearish</div>
          </div>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: '1.8rem', fontWeight: 700, color: '#eab308' }}>{patterns.summary?.neutral || 0}</div>
            <div style={{ fontSize: '0.75rem', color: '#94a3b8' }}>Neutral</div>
          </div>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: '1.8rem', fontWeight: 700, color: '#3b82f6' }}>{patterns.total_patterns || 0}</div>
            <div style={{ fontSize: '0.75rem', color: '#94a3b8' }}>Total</div>
          </div>
        </div>
      </div>

      <div className="pattern-grid">
        {(patterns.patterns || []).slice(0, 50).map((pattern, i) => (
          <div key={i} className={`pattern-card ${pattern.type}`}>
            <div className="pattern-name">{pattern.name}</div>
            <div className="pattern-desc">{pattern.description}</div>
            <div className="pattern-meta">
              <span style={{
                color: pattern.type === 'bullish' ? '#22c55e' : pattern.type === 'bearish' ? '#ef4444' : '#eab308',
                textTransform: 'uppercase',
                fontWeight: 600,
              }}>
                {pattern.type}
              </span>
              <span>Confidence: {(pattern.confidence * 100).toFixed(0)}%</span>
              <span>Candle #{pattern.index}</span>
            </div>
            <div className="confidence-bar" style={{ marginTop: 8 }}>
              <div
                className="confidence-fill"
                style={{
                  width: `${pattern.confidence * 100}%`,
                  background: pattern.type === 'bullish' ? '#22c55e' : pattern.type === 'bearish' ? '#ef4444' : '#eab308',
                }}
              />
            </div>
          </div>
        ))}
      </div>

      {(!patterns.patterns || patterns.patterns.length === 0) && (
        <div className="card" style={{ textAlign: 'center', padding: 40, color: '#64748b' }}>
          No patterns detected in the current timeframe.
        </div>
      )}
    </div>
  );
}

export default PatternPanel;
