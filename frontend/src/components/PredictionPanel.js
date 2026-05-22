import React, { useState, useEffect } from 'react';
import axios from 'axios';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

function PredictionPanel({ prediction, symbol }) {
  const [multiPred, setMultiPred] = useState(null);
  const [kronosPred, setKronosPred] = useState(null);
  const [kronosLoading, setKronosLoading] = useState(false);
  const [steps, setSteps] = useState(7);
  const [activeModel, setActiveModel] = useState('kronos');

  // Fetch ensemble multi-step predictions
  useEffect(() => {
    const fetchMulti = async () => {
      try {
        const res = await axios.get(`${API}/predictions/${symbol}/multi?steps=${steps}`);
        setMultiPred(res.data.predictions);
      } catch (e) { /* ignore */ }
    };
    fetchMulti();
  }, [symbol, steps]);

  // Fetch Kronos predictions
  useEffect(() => {
    const fetchKronos = async () => {
      setKronosLoading(true);
      try {
        const res = await axios.get(`${API}/predictions/${symbol}/kronos?steps=${steps > 10 ? 10 : steps}`);
        setKronosPred(res.data);
      } catch (e) { /* ignore */ }
      setKronosLoading(false);
    };
    fetchKronos();
  }, [symbol, steps]);

  if (!prediction) return <div className="card"><p>Loading prediction...</p></div>;
  if (prediction.error) return (
    <div className="card"><p style={{ color: '#ef4444' }}>{prediction.error}</p></div>
  );

  const isUp = prediction.change_percent > 0;
  const activePreds = activeModel === 'kronos' ? kronosPred?.predictions : multiPred;
  const isKronos = activeModel === 'kronos';

  return (
    <div>
      {/* Model Toggle */}
      <div className="card" style={{ marginBottom: 16 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
          <div className="card-title" style={{ marginBottom: 0 }}>Prediction — {symbol}</div>
          <div style={{ display: 'flex', gap: 4 }}>
            <button
              onClick={() => setActiveModel('kronos')}
              style={{
                padding: '6px 14px', borderRadius: 4, border: 'none', cursor: 'pointer',
                fontSize: '0.8rem', fontWeight: 600,
                background: activeModel === 'kronos' ? '#a855f7' : '#1e293b',
                color: activeModel === 'kronos' ? 'white' : '#94a3b8',
              }}
            >
              Kronos AI
            </button>
            <button
              onClick={() => setActiveModel('ensemble')}
              style={{
                padding: '6px 14px', borderRadius: 4, border: 'none', cursor: 'pointer',
                fontSize: '0.8rem', fontWeight: 600,
                background: activeModel === 'ensemble' ? '#3b82f6' : '#1e293b',
                color: activeModel === 'ensemble' ? 'white' : '#94a3b8',
              }}
            >
              Ensemble ML
            </button>
          </div>
        </div>

        {/* Model info banner */}
        {isKronos ? (
          <div style={{
            padding: '10px 14px', borderRadius: 6, marginBottom: 16,
            background: 'rgba(168,85,247,0.08)', border: '1px solid rgba(168,85,247,0.2)',
          }}>
            <div style={{ fontSize: '0.9rem', fontWeight: 700, color: '#a855f7', marginBottom: 4 }}>
              Kronos Foundation Model
            </div>
            <div style={{ fontSize: '0.75rem', color: '#94a3b8', lineHeight: 1.5 }}>
              Decoder-only Transformer pre-trained on 12B K-lines from 45+ global exchanges.
              Hierarchical tokenization with Binary Spherical Quantization. 24.7M parameters.
            </div>
            {kronosPred && (
              <div style={{ display: 'flex', gap: 16, marginTop: 8, fontSize: '0.75rem', color: '#64748b' }}>
                <span>T={kronosPred.parameters?.temperature}</span>
                <span>top_p={kronosPred.parameters?.top_p}</span>
                <span>samples={kronosPred.parameters?.sample_count}</span>
                <span>lookback={kronosPred.parameters?.lookback}</span>
              </div>
            )}
          </div>
        ) : (
          <div style={{
            padding: '10px 14px', borderRadius: 6, marginBottom: 16,
            background: 'rgba(59,130,246,0.08)', border: '1px solid rgba(59,130,246,0.2)',
          }}>
            <div style={{ fontSize: '0.9rem', fontWeight: 700, color: '#3b82f6', marginBottom: 4 }}>
              Ensemble ML (XGBoost + GradientBoost + RandomForest)
            </div>
            <div style={{ fontSize: '0.75rem', color: '#94a3b8' }}>
              Traditional ML ensemble with 20 technical indicator features. Lookback window of 10 candles.
            </div>
          </div>
        )}

        {/* Summary stats */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 16 }}>
          {isKronos && kronosPred && !kronosPred.error ? (
            <>
              <span style={{
                fontSize: '2rem', fontWeight: 700,
                color: kronosPred.direction === 'bullish' ? '#22c55e' : '#ef4444',
              }}>
                {kronosPred.direction === 'bullish' ? '▲' : '▼'} {kronosPred.direction?.toUpperCase()}
              </span>
              <span style={{ fontSize: '1.5rem', fontWeight: 600 }}>
                {kronosPred.total_change_percent > 0 ? '+' : ''}{kronosPred.total_change_percent}%
              </span>
              <span style={{ fontSize: '0.85rem', color: '#94a3b8' }}>
                ${kronosPred.current_close} → ${kronosPred.final_predicted_close}
              </span>
            </>
          ) : isKronos && kronosLoading ? (
            <span style={{ color: '#a855f7' }}>Loading Kronos model...</span>
          ) : isKronos && kronosPred?.error ? (
            <span style={{ color: '#ef4444' }}>{kronosPred.error}</span>
          ) : (
            <>
              <span style={{
                fontSize: '2rem', fontWeight: 700,
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
            </>
          )}
        </div>

        {/* OHLC cards (ensemble next-day) */}
        {!isKronos && (
          <>
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
          </>
        )}
      </div>

      {/* Multi-step table */}
      <div className="card">
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <div className="card-title">
            {isKronos ? 'Kronos Multi-Day Forecast' : 'Ensemble Multi-Step Forecast'}
          </div>
          <select
            value={steps}
            onChange={(e) => setSteps(parseInt(e.target.value))}
            className="select"
            style={{ fontSize: '0.8rem', padding: '4px 8px' }}
          >
            {[3, 5, 7, 10].map(n => (
              <option key={n} value={n}>{n} days</option>
            ))}
          </select>
        </div>

        {activePreds && activePreds.length > 0 ? (
          <div style={{ overflowX: 'auto' }}>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem', marginTop: 8 }}>
              <thead>
                <tr style={{ color: '#64748b', textAlign: 'left' }}>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Day</th>
                  {isKronos && <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Date</th>}
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Open</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>High</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Low</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Close</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Change</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Direction</th>
                </tr>
              </thead>
              <tbody>
                {activePreds.map((p, i) => (
                  <tr key={i} style={{ borderBottom: '1px solid #1e293b' }}>
                    <td style={{ padding: '8px 12px' }}>Day {p.step}</td>
                    {isKronos && <td style={{ padding: '8px 12px', color: '#64748b', fontSize: '0.8rem' }}>{p.date}</td>}
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
                        padding: '2px 8px', borderRadius: 4, fontSize: '0.75rem', fontWeight: 600,
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
        ) : (
          <div style={{ padding: 20, textAlign: 'center', color: '#64748b' }}>
            {kronosLoading ? 'Loading Kronos predictions...' : 'No predictions available'}
          </div>
        )}
      </div>
    </div>
  );
}

export default PredictionPanel;
