import React, { useState, useEffect } from 'react';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

// A/B experiment scoreboard: the real trader vs every shadow model (incl. exp1).
function ExperimentsPanel() {
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const res = await fetch(`${API}/experiments`);
        const json = await res.json();
        if (active) { setData(json); setError(null); }
      } catch (e) {
        if (active) setError('Backend not reachable');
      }
    };
    load();
    const id = setInterval(load, 10000);
    return () => { active = false; clearInterval(id); };
  }, []);

  if (error) return <div style={S.wrap}><div style={S.err}>{error}</div></div>;
  if (!data) return <div style={S.wrap}><div style={S.muted}>Loading experiments…</div></div>;

  const models = data.models || [];
  const col = (v) => (v >= 0 ? '#16a34a' : '#dc2626');

  return (
    <div style={S.wrap}>
      <div style={S.title}>A/B Experiments</div>
      <div style={S.sub}>
        All models run in parallel on the same live prices (paper only). exp1 is the
        short-horizon prediction trader — it buys when the next-minute forecast predicts
        an up-move and holds ~5 minutes.
      </div>

      <table style={S.table}>
        <thead>
          <tr>
            {['Model', 'Value', 'Realized P&L', 'Trades', 'Win rate', 'Open', 'What it tests'].map((h) => (
              <th key={h} style={S.th}>{h}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {models.map((m) => {
            const isExp = m.kind === 'experiment';
            const isReal = m.kind === 'real';
            return (
              <tr key={m.model_id} style={{
                background: isExp ? 'rgba(37,99,235,0.15)' : isReal ? 'rgba(22,163,74,0.10)' : 'transparent',
              }}>
                <td style={{ ...S.td, fontWeight: 700 }}>
                  {m.model_id}
                  {isExp && <span style={S.tagExp}> EXPERIMENT</span>}
                  {isReal && <span style={S.tagReal}> REAL</span>}
                </td>
                <td style={S.td}>${(m.portfolio_value ?? 0).toFixed(2)}</td>
                <td style={{ ...S.td, color: col(m.realized_pnl ?? 0), fontWeight: 600 }}>
                  {(m.realized_pnl ?? 0) >= 0 ? '+' : ''}{(m.realized_pnl ?? 0).toFixed(2)}
                </td>
                <td style={S.td}>{m.total_trades}</td>
                <td style={S.td}>{(m.win_rate_pct ?? 0).toFixed(1)}%</td>
                <td style={S.td}>{m.open_positions}</td>
                <td style={{ ...S.td, color: '#9ca3af', fontSize: 11.5 }}>{m.description}</td>
              </tr>
            );
          })}
        </tbody>
      </table>

      <div style={S.note}>
        Judge models by <b>realized P&L per trade</b> over many trades, not by a lucky day.
        The random baseline is the bar: any model that can't beat it has no real skill.
      </div>
    </div>
  );
}

const S = {
  wrap: { padding: 20, maxWidth: 1080, margin: '0 auto' },
  title: { fontSize: 22, fontWeight: 700, color: '#f3f4f6' },
  sub: { fontSize: 13, color: '#9ca3af', margin: '6px 0 16px 0', lineHeight: 1.5 },
  table: { width: '100%', borderCollapse: 'collapse', background: '#111827', borderRadius: 10, overflow: 'hidden' },
  th: { textAlign: 'left', padding: '10px 12px', fontSize: 11, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.5, borderBottom: '1px solid #374151' },
  td: { padding: '10px 12px', fontSize: 13, color: '#e5e7eb', borderBottom: '1px solid #1f2937' },
  tagExp: { fontSize: 9, fontWeight: 800, color: '#60a5fa', marginLeft: 6 },
  tagReal: { fontSize: 9, fontWeight: 800, color: '#34d399', marginLeft: 6 },
  note: { fontSize: 12, color: '#6b7280', lineHeight: 1.5, background: '#111827', padding: 12, borderRadius: 8, border: '1px solid #1f2937', marginTop: 14 },
  muted: { color: '#9ca3af', padding: 40, textAlign: 'center' },
  err: { color: '#dc2626', padding: 40, textAlign: 'center' },
};

export default ExperimentsPanel;
