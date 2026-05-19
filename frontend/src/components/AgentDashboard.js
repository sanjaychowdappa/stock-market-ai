import React, { useState, useEffect } from 'react';
import axios from 'axios';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

function AgentDashboard({ analysis, symbol }) {
  const [rankings, setRankings] = useState([]);

  useEffect(() => {
    const fetchRankings = async () => {
      try {
        const res = await axios.get(`${API}/signals/rankings/top`);
        setRankings(res.data.rankings || []);
      } catch (e) { /* ignore */ }
    };
    fetchRankings();
  }, []);

  const rec = analysis?.agent_recommendation;
  const signals = analysis?.signals;
  const prediction = analysis?.prediction;

  return (
    <div>
      {rec && (
        <div className="card" style={{ marginBottom: 16 }}>
          <div className="card-title">AI Agent Analysis - {symbol}</div>

          <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 16 }}>
            <span style={{
              fontSize: '2rem',
              fontWeight: 700,
              color: rec.action.includes('BUY') ? '#22c55e' : rec.action.includes('SELL') ? '#ef4444' : '#eab308',
            }}>
              {rec.action}
            </span>
            <div>
              <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Composite Score</div>
              <div style={{ fontSize: '1.2rem', fontWeight: 600 }}>{rec.score}</div>
            </div>
          </div>

          <p style={{ fontSize: '0.9rem', color: '#94a3b8', lineHeight: 1.6, marginBottom: 16 }}>
            {rec.summary}
          </p>

          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12 }}>
            <div className="sl-card">
              <div className="sl-label">Signal Score</div>
              <div style={{
                fontSize: '1.2rem', fontWeight: 700,
                color: (signals?.score || 0) > 0 ? '#22c55e' : '#ef4444',
              }}>
                {signals?.score || 'N/A'}
              </div>
            </div>
            <div className="sl-card">
              <div className="sl-label">Predicted Change</div>
              <div style={{
                fontSize: '1.2rem', fontWeight: 700,
                color: (prediction?.change_percent || 0) > 0 ? '#22c55e' : '#ef4444',
              }}>
                {prediction?.change_percent ? `${prediction.change_percent}%` : 'N/A'}
              </div>
            </div>
            <div className="sl-card">
              <div className="sl-label">Recent Patterns</div>
              <div style={{ fontSize: '1.2rem', fontWeight: 700, color: '#3b82f6' }}>
                {analysis?.patterns?.length || 0}
              </div>
            </div>
          </div>
        </div>
      )}

      {signals?.individual_signals && (
        <div className="card" style={{ marginBottom: 16 }}>
          <div className="card-title">Individual Indicator Signals</div>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ color: '#64748b', textAlign: 'left' }}>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Indicator</th>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Score</th>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Weight</th>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Signal</th>
              </tr>
            </thead>
            <tbody>
              {signals.individual_signals.map((sig, i) => (
                <tr key={i} style={{ borderBottom: '1px solid #1e293b' }}>
                  <td style={{ padding: '8px 12px', fontWeight: 500 }}>{sig.name}</td>
                  <td style={{
                    padding: '8px 12px',
                    color: sig.score > 0 ? '#22c55e' : sig.score < 0 ? '#ef4444' : '#94a3b8',
                    fontWeight: 600,
                  }}>
                    {sig.score > 0 ? '+' : ''}{sig.score.toFixed(2)}
                  </td>
                  <td style={{ padding: '8px 12px', color: '#94a3b8' }}>{sig.weight?.toFixed(1)}</td>
                  <td style={{ padding: '8px 12px', color: '#94a3b8', fontSize: '0.8rem' }}>
                    {sig.reason || '-'}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div className="card">
        <div className="card-title">Opportunity Rankings (All Watchlist)</div>
        {rankings.length > 0 ? (
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ color: '#64748b', textAlign: 'left' }}>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>#</th>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Symbol</th>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Action</th>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Score</th>
                <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Price</th>
              </tr>
            </thead>
            <tbody>
              {rankings.map((r, i) => (
                <tr key={i} style={{ borderBottom: '1px solid #1e293b' }}>
                  <td style={{ padding: '8px 12px' }}>{i + 1}</td>
                  <td style={{ padding: '8px 12px', fontWeight: 600 }}>{r.symbol}</td>
                  <td style={{ padding: '8px 12px' }}>
                    <span style={{
                      padding: '2px 8px',
                      borderRadius: 4,
                      fontSize: '0.75rem',
                      fontWeight: 600,
                      background: r.action.includes('BUY') ? 'rgba(34,197,94,0.2)' : r.action.includes('SELL') ? 'rgba(239,68,68,0.2)' : 'rgba(234,179,8,0.2)',
                      color: r.action.includes('BUY') ? '#22c55e' : r.action.includes('SELL') ? '#ef4444' : '#eab308',
                    }}>
                      {r.action}
                    </span>
                  </td>
                  <td style={{
                    padding: '8px 12px',
                    fontWeight: 600,
                    color: r.score > 0 ? '#22c55e' : r.score < 0 ? '#ef4444' : '#94a3b8',
                  }}>
                    {r.score > 0 ? '+' : ''}{r.score.toFixed(3)}
                  </td>
                  <td style={{ padding: '8px 12px' }}>${r.current_price}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p style={{ color: '#64748b', fontSize: '0.85rem', padding: '16px 0' }}>
            Rankings will appear after the agent completes its first analysis cycle (runs every 5 minutes).
          </p>
        )}
      </div>
    </div>
  );
}

export default AgentDashboard;
