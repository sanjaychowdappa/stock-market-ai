import React from 'react';

function StopLossReport({ report, symbol }) {
  if (!report) return <div className="card"><p>Loading stop loss report...</p></div>;

  return (
    <div>
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">Stop Loss Report - {symbol}</div>

        <div style={{ display: 'flex', gap: 24, marginBottom: 16 }}>
          <div>
            <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Entry Price</div>
            <div style={{ fontSize: '1.3rem', fontWeight: 700 }}>${report.entry_price}</div>
          </div>
          <div>
            <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Current Price</div>
            <div style={{ fontSize: '1.3rem', fontWeight: 700 }}>${report.current_price}</div>
          </div>
          <div>
            <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Position</div>
            <div style={{ fontSize: '1.3rem', fontWeight: 700, textTransform: 'capitalize' }}>{report.position_type}</div>
          </div>
          <div>
            <div style={{ fontSize: '0.75rem', color: '#64748b' }}>ATR (14)</div>
            <div style={{ fontSize: '1.3rem', fontWeight: 700 }}>${report.atr}</div>
          </div>
          <div>
            <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Trailing Stop</div>
            <div style={{ fontSize: '1.3rem', fontWeight: 700, color: '#eab308' }}>${report.trailing_stop}</div>
          </div>
        </div>
      </div>

      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">Stop Loss Levels</div>
        <div className="stop-loss-grid">
          {Object.entries(report.stop_levels || {}).map(([key, value]) => (
            <div key={key} className="sl-card">
              <div className="sl-label">{key.replace(/_/g, ' ')}</div>
              <div className="sl-value">${value}</div>
              {report.risk_per_share?.[key] && (
                <div style={{ fontSize: '0.75rem', color: '#94a3b8', marginTop: 4 }}>
                  Risk: ${report.risk_per_share[key]}/share
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
        <div className="card">
          <div className="card-title">Support Levels</div>
          {(report.support_levels || []).length > 0 ? (
            report.support_levels.map((level, i) => (
              <div key={i} className="stat-row">
                <span className="stat-label">S{i + 1}</span>
                <span className="stat-value" style={{ color: '#22c55e' }}>${level}</span>
              </div>
            ))
          ) : (
            <p style={{ color: '#64748b', fontSize: '0.85rem' }}>No support levels identified</p>
          )}
        </div>

        <div className="card">
          <div className="card-title">Resistance Levels</div>
          {(report.resistance_levels || []).length > 0 ? (
            report.resistance_levels.map((level, i) => (
              <div key={i} className="stat-row">
                <span className="stat-label">R{i + 1}</span>
                <span className="stat-value" style={{ color: '#ef4444' }}>${level}</span>
              </div>
            ))
          ) : (
            <p style={{ color: '#64748b', fontSize: '0.85rem' }}>No resistance levels identified</p>
          )}
        </div>
      </div>

      {report.recommendation && (
        <div className="card" style={{ marginTop: 16, borderLeft: '3px solid #3b82f6' }}>
          <div className="card-title">AI Recommendation</div>
          <p style={{ fontSize: '0.9rem', color: '#e2e8f0', lineHeight: 1.6 }}>{report.recommendation}</p>
        </div>
      )}
    </div>
  );
}

export default StopLossReport;
