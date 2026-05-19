import React, { useState, useEffect } from 'react';
import axios from 'axios';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

function PredictionPanel({ prediction, symbol }) {
  const [multiPred, setMultiPred] = useState(null);
  const [steps, setSteps] = useState(5);

  useEffect(() => {
    const fetchMulti = async () => {
      try {
        const res = await axios.get(`${API}/predictions/${symbol}/multi?steps=${steps}`);
        setMultiPred(res.data.predictions);
      } catch (e) { /* ignore */ }
    };
    fetchMulti();
  }, [symbol, steps]);

  if (!prediction) return <div className="card"><p>Loading prediction...</p></div>;

  if (prediction.error) return (
    <div className="card"><p style={{ color: '#ef4444' }}>{prediction.error}</p></div>
  );

  const isUp = prediction.change_percent > 0;

  return (
    <div>
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">Next Candle Prediction - {symbol}</div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 16 }}>
          <span style={{
            fontSize: '2rem',
            fontWeight: 700,
            color: isUp ? '#22c55e' : '#ef4444',
          }}>
            {isUp ? '▲' : '▼'} {prediction.direction?.toUpperCase()}
          </span>
          <span style={{ fontSize: '1.5rem', fontWeight: 600 }}>
            {prediction.change_percent > 0 ? '+' : ''}{prediction.change_percent}%
          </span>
          <div className="confidence-bar" style={{ width: 150 }}>
            <div
              className="confidence-fill"
              style={{
                width: `${prediction.confidence * 100}%`,
                background: prediction.confidence > 0.6 ? '#22c55e' : prediction.confidence > 0.4 ? '#eab308' : '#ef4444',
              }}
            />
          </div>
          <span style={{ fontSize: '0.85rem', color: '#94a3b8' }}>
            {(prediction.confidence * 100).toFixed(0)}% confidence
          </span>
        </div>

        <div className="prediction-candle">
          <div className="pred-item">
            <div className="pred-label">Open</div>
            <div className="pred-value">${prediction.predicted_open}</div>
          </div>
          <div className="pred-item">
            <div className="pred-label">High</div>
            <div className="pred-value" style={{ color: '#22c55e' }}>${prediction.predicted_high}</div>
          </div>
          <div className="pred-item">
            <div className="pred-label">Low</div>
            <div className="pred-value" style={{ color: '#ef4444' }}>${prediction.predicted_low}</div>
          </div>
          <div className="pred-item">
            <div className="pred-label">Close</div>
            <div className="pred-value" style={{ color: isUp ? '#22c55e' : '#ef4444' }}>
              ${prediction.predicted_close}
            </div>
          </div>
        </div>

        <div style={{ marginTop: 12, fontSize: '0.8rem', color: '#64748b' }}>
          Models: {prediction.models_used?.join(', ')} | Current: ${prediction.current_close}
        </div>
      </div>

      <div className="card">
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <div className="card-title">Multi-Step Forecast</div>
          <select
            value={steps}
            onChange={(e) => setSteps(parseInt(e.target.value))}
            className="select"
            style={{ fontSize: '0.8rem', padding: '4px 8px' }}
          >
            {[3, 5, 7, 10].map(n => (
              <option key={n} value={n}>{n} steps</option>
            ))}
          </select>
        </div>

        {multiPred && (
          <div style={{ overflowX: 'auto' }}>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem', marginTop: 8 }}>
              <thead>
                <tr style={{ color: '#64748b', textAlign: 'left' }}>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Step</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Open</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>High</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Low</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Close</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Change</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Direction</th>
                </tr>
              </thead>
              <tbody>
                {multiPred.map((p, i) => (
                  <tr key={i} style={{ borderBottom: '1px solid #1e293b' }}>
                    <td style={{ padding: '8px 12px' }}>Day {p.step}</td>
                    <td style={{ padding: '8px 12px' }}>${p.predicted_open}</td>
                    <td style={{ padding: '8px 12px', color: '#22c55e' }}>${p.predicted_high}</td>
                    <td style={{ padding: '8px 12px', color: '#ef4444' }}>${p.predicted_low}</td>
                    <td style={{ padding: '8px 12px', fontWeight: 600 }}>${p.predicted_close}</td>
                    <td style={{
                      padding: '8px 12px',
                      color: p.change_percent > 0 ? '#22c55e' : '#ef4444',
                      fontWeight: 600,
                    }}>
                      {p.change_percent > 0 ? '+' : ''}{p.change_percent}%
                    </td>
                    <td style={{ padding: '8px 12px' }}>
                      <span style={{
                        padding: '2px 8px',
                        borderRadius: 4,
                        fontSize: '0.75rem',
                        fontWeight: 600,
                        background: p.direction === 'bullish' ? 'rgba(34,197,94,0.2)' : 'rgba(239,68,68,0.2)',
                        color: p.direction === 'bullish' ? '#22c55e' : '#ef4444',
                      }}>
                        {p.direction}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}

export default PredictionPanel;
