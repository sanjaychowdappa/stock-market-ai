import React, { useState, useEffect } from 'react';
import axios from 'axios';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

function NewsReport({ symbol }) {
  const [report, setReport] = useState(null);
  const [watchlistReport, setWatchlistReport] = useState(null);
  const [view, setView] = useState('stock');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    const fetchReport = async () => {
      setLoading(true);
      try {
        const res = await axios.get(`${API}/news/${symbol}/report`);
        setReport(res.data);
      } catch (e) { /* silent */ }
      setLoading(false);
    };
    fetchReport();
  }, [symbol]);

  const loadWatchlistReport = async () => {
    setView('watchlist');
    if (watchlistReport) return;
    setLoading(true);
    try {
      const res = await axios.get(`${API}/news/watchlist/report`);
      setWatchlistReport(res.data);
    } catch (e) { /* silent */ }
    setLoading(false);
  };

  const rec = report?.recommendation;
  const sentiment = report?.news_sentiment;

  return (
    <div>
      <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
        <button
          className={`tab ${view === 'stock' ? 'active' : ''}`}
          onClick={() => setView('stock')}
          style={{ borderRadius: 6, border: '1px solid #1e293b' }}
        >
          {symbol} Report
        </button>
        <button
          className={`tab ${view === 'watchlist' ? 'active' : ''}`}
          onClick={loadWatchlistReport}
          style={{ borderRadius: 6, border: '1px solid #1e293b' }}
        >
          Watchlist Report
        </button>
      </div>

      {loading && <div style={{ textAlign: 'center', padding: 40 }}><div className="spinner" /></div>}

      {view === 'stock' && report && !loading && (
        <div>
          {rec && (
            <div className="card" style={{ marginBottom: 16, borderLeft: `3px solid ${getActionColor(rec.action)}` }}>
              <div className="card-title">AI Agent Recommendation</div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 16 }}>
                <span style={{ fontSize: '2rem', fontWeight: 700, color: getActionColor(rec.action) }}>
                  {rec.action}
                </span>
                <div>
                  <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Composite Score</div>
                  <div style={{ fontSize: '1.2rem', fontWeight: 600 }}>{rec.composite_score}</div>
                </div>
                <div>
                  <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Risk Level</div>
                  <div style={{
                    fontSize: '0.9rem', fontWeight: 600,
                    color: rec.risk_level === 'low' ? '#22c55e' : rec.risk_level === 'high' ? '#ef4444' : '#eab308'
                  }}>
                    {rec.risk_level?.toUpperCase()}
                  </div>
                </div>
                <div>
                  <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Signal Alignment</div>
                  <div style={{
                    fontSize: '0.9rem', fontWeight: 600,
                    color: rec.signal_alignment === 'aligned' ? '#22c55e' : rec.signal_alignment === 'conflicting' ? '#ef4444' : '#eab308'
                  }}>
                    {rec.signal_alignment?.toUpperCase()}
                  </div>
                </div>
              </div>

              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12, marginBottom: 16 }}>
                {Object.entries(rec.component_scores || {}).map(([key, val]) => (
                  <div key={key} className="sl-card">
                    <div className="sl-label">{key} ({((rec.component_weights || {})[key] * 100).toFixed(0)}%)</div>
                    <div style={{ fontSize: '1.1rem', fontWeight: 700, color: val > 0 ? '#22c55e' : val < 0 ? '#ef4444' : '#94a3b8' }}>
                      {val > 0 ? '+' : ''}{val}
                    </div>
                  </div>
                ))}
              </div>

              <div style={{ marginBottom: 12 }}>
                <div style={{ fontSize: '0.75rem', color: '#64748b', marginBottom: 6 }}>REASONING</div>
                <ul className="reason-list">
                  {(rec.reasons || []).map((r, i) => <li key={i}>{r}</li>)}
                </ul>
              </div>

              {rec.stop_loss && (
                <div style={{ display: 'flex', gap: 16, fontSize: '0.85rem' }}>
                  <span style={{ color: '#64748b' }}>Stop Loss: <strong style={{ color: '#ef4444' }}>${rec.stop_loss}</strong></span>
                  {(rec.take_profit_targets || []).map((tp, i) => (
                    <span key={i} style={{ color: '#64748b' }}>TP{i + 1}: <strong style={{ color: '#22c55e' }}>${tp}</strong></span>
                  ))}
                </div>
              )}
            </div>
          )}

          {sentiment && (
            <div className="card" style={{ marginBottom: 16 }}>
              <div className="card-title">News Sentiment Analysis</div>
              <div style={{ display: 'flex', gap: 24, marginBottom: 16 }}>
                <div style={{ textAlign: 'center' }}>
                  <div style={{ fontSize: '1.5rem', fontWeight: 700, color: getSentimentColor(sentiment.overall_sentiment) }}>
                    {sentiment.overall_sentiment?.toUpperCase()}
                  </div>
                  <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Overall</div>
                </div>
                <div style={{ textAlign: 'center' }}>
                  <div style={{ fontSize: '1.5rem', fontWeight: 700 }}>{sentiment.overall_score}</div>
                  <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Score</div>
                </div>
                <div style={{ display: 'flex', gap: 12 }}>
                  <div style={{ textAlign: 'center' }}>
                    <div style={{ fontSize: '1.3rem', fontWeight: 700, color: '#22c55e' }}>{sentiment.breakdown?.bullish || 0}</div>
                    <div style={{ fontSize: '0.7rem', color: '#64748b' }}>Bullish</div>
                  </div>
                  <div style={{ textAlign: 'center' }}>
                    <div style={{ fontSize: '1.3rem', fontWeight: 700, color: '#ef4444' }}>{sentiment.breakdown?.bearish || 0}</div>
                    <div style={{ fontSize: '0.7rem', color: '#64748b' }}>Bearish</div>
                  </div>
                  <div style={{ textAlign: 'center' }}>
                    <div style={{ fontSize: '1.3rem', fontWeight: 700, color: '#eab308' }}>{sentiment.breakdown?.neutral || 0}</div>
                    <div style={{ fontSize: '0.7rem', color: '#64748b' }}>Neutral</div>
                  </div>
                </div>
              </div>

              {(sentiment.article_sentiments || []).map((a, i) => (
                <div key={i} style={{
                  padding: '10px 0',
                  borderBottom: '1px solid #1e293b',
                  display: 'flex',
                  gap: 12,
                  alignItems: 'flex-start',
                }}>
                  <span style={{
                    padding: '2px 8px',
                    borderRadius: 4,
                    fontSize: '0.7rem',
                    fontWeight: 700,
                    flexShrink: 0,
                    background: a.label === 'bullish' ? 'rgba(34,197,94,0.2)' : a.label === 'bearish' ? 'rgba(239,68,68,0.2)' : 'rgba(234,179,8,0.2)',
                    color: getSentimentColor(a.label),
                  }}>
                    {a.score > 0 ? '+' : ''}{a.score}
                  </span>
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: '0.85rem', fontWeight: 500, marginBottom: 2 }}>
                      {a.url ? <a href={a.url} target="_blank" rel="noopener noreferrer" style={{ color: '#e2e8f0', textDecoration: 'none' }}>{a.title}</a> : a.title}
                    </div>
                    <div style={{ fontSize: '0.75rem', color: '#64748b' }}>
                      {a.provider} {a.published && `· ${new Date(a.published).toLocaleDateString()}`}
                      {a.bull_words?.length > 0 && <span style={{ color: '#22c55e' }}> +{a.bull_words.join(', ')}</span>}
                      {a.bear_words?.length > 0 && <span style={{ color: '#ef4444' }}> -{a.bear_words.join(', ')}</span>}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {view === 'watchlist' && watchlistReport && !loading && (
        <div>
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="card-title">Market Overview</div>
            <div style={{ display: 'flex', gap: 24 }}>
              <div style={{ textAlign: 'center' }}>
                <div style={{ fontSize: '1.5rem', fontWeight: 700 }}>{watchlistReport.market_summary?.mood}</div>
                <div style={{ fontSize: '0.75rem', color: '#64748b' }}>Market Mood</div>
              </div>
              <div style={{ display: 'flex', gap: 16 }}>
                <div style={{ textAlign: 'center' }}>
                  <div style={{ fontSize: '1.3rem', fontWeight: 700, color: '#22c55e' }}>{watchlistReport.market_summary?.buys || 0}</div>
                  <div style={{ fontSize: '0.7rem', color: '#64748b' }}>Buys</div>
                </div>
                <div style={{ textAlign: 'center' }}>
                  <div style={{ fontSize: '1.3rem', fontWeight: 700, color: '#ef4444' }}>{watchlistReport.market_summary?.sells || 0}</div>
                  <div style={{ fontSize: '0.7rem', color: '#64748b' }}>Sells</div>
                </div>
                <div style={{ textAlign: 'center' }}>
                  <div style={{ fontSize: '1.3rem', fontWeight: 700, color: '#eab308' }}>{watchlistReport.market_summary?.holds || 0}</div>
                  <div style={{ fontSize: '0.7rem', color: '#64748b' }}>Holds</div>
                </div>
              </div>
            </div>
          </div>

          {watchlistReport.top_buys?.length > 0 && (
            <div className="card" style={{ marginBottom: 16, borderLeft: '3px solid #22c55e' }}>
              <div className="card-title">Top Buy Picks</div>
              {watchlistReport.top_buys.map((s, i) => (
                <StockRow key={i} stock={s} rank={i + 1} />
              ))}
            </div>
          )}

          {watchlistReport.top_sells?.length > 0 && (
            <div className="card" style={{ marginBottom: 16, borderLeft: '3px solid #ef4444' }}>
              <div className="card-title">Top Sell Picks</div>
              {watchlistReport.top_sells.map((s, i) => (
                <StockRow key={i} stock={s} rank={i + 1} />
              ))}
            </div>
          )}

          <div className="card">
            <div className="card-title">All Stocks</div>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
              <thead>
                <tr style={{ color: '#64748b', textAlign: 'left' }}>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Symbol</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Action</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Score</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>News</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Technical</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Risk</th>
                  <th style={{ padding: '8px 12px', borderBottom: '1px solid #1e293b' }}>Price</th>
                </tr>
              </thead>
              <tbody>
                {(watchlistReport.all_stocks || []).map((s, i) => (
                  <tr key={i} style={{ borderBottom: '1px solid #1e293b' }}>
                    <td style={{ padding: '8px 12px', fontWeight: 600 }}>{s.symbol}</td>
                    <td style={{ padding: '8px 12px' }}>
                      <span style={{
                        padding: '2px 8px', borderRadius: 4, fontSize: '0.75rem', fontWeight: 600,
                        background: getActionBg(s.action), color: getActionColor(s.action),
                      }}>{s.action}</span>
                    </td>
                    <td style={{ padding: '8px 12px', fontWeight: 600, color: (s.composite_score || 0) > 0 ? '#22c55e' : (s.composite_score || 0) < 0 ? '#ef4444' : '#94a3b8' }}>
                      {(s.composite_score || 0) > 0 ? '+' : ''}{s.composite_score}
                    </td>
                    <td style={{ padding: '8px 12px', color: getSentimentColor(s.news_sentiment) }}>
                      {s.news_sentiment}
                    </td>
                    <td style={{ padding: '8px 12px' }}>{s.tech_signal}</td>
                    <td style={{ padding: '8px 12px', color: s.risk_level === 'low' ? '#22c55e' : s.risk_level === 'high' ? '#ef4444' : '#eab308' }}>
                      {s.risk_level}
                    </td>
                    <td style={{ padding: '8px 12px' }}>${s.price}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}

function StockRow({ stock, rank }) {
  return (
    <div style={{ padding: '10px 0', borderBottom: '1px solid #1e293b', display: 'flex', alignItems: 'center', gap: 16 }}>
      <span style={{ fontSize: '1.2rem', fontWeight: 700, color: '#64748b', width: 30 }}>#{rank}</span>
      <span style={{ fontWeight: 700, width: 60 }}>{stock.symbol}</span>
      <span style={{
        padding: '2px 10px', borderRadius: 4, fontSize: '0.8rem', fontWeight: 700,
        background: getActionBg(stock.action), color: getActionColor(stock.action),
      }}>{stock.action}</span>
      <span style={{ fontSize: '0.85rem', color: '#94a3b8', flex: 1 }}>
        {(stock.reasons || []).join(' | ')}
      </span>
      <span style={{ fontWeight: 600 }}>${stock.price}</span>
    </div>
  );
}

function getActionColor(action) {
  if (!action) return '#94a3b8';
  if (action.includes('BUY')) return '#22c55e';
  if (action.includes('SELL')) return '#ef4444';
  return '#eab308';
}

function getActionBg(action) {
  if (!action) return 'rgba(148,163,184,0.2)';
  if (action.includes('BUY')) return 'rgba(34,197,94,0.2)';
  if (action.includes('SELL')) return 'rgba(239,68,68,0.2)';
  return 'rgba(234,179,8,0.2)';
}

function getSentimentColor(label) {
  if (label === 'bullish') return '#22c55e';
  if (label === 'bearish') return '#ef4444';
  return '#eab308';
}

export default NewsReport;
