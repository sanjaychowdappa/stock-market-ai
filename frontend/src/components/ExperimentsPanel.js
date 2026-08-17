import React, { useState, useEffect } from 'react';
import SimBanner from './SimBanner';

const API = process.env.REACT_APP_API_URL || 'http://127.0.0.1:8000/api';

const fmtHold = (s) => {
  if (s == null) return '';
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${s % 60 > 0 ? ` ${s % 60}s` : ''}`;
};


/* ─── A/B scoreboard ─────────────────────────────────────────────── */
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
      <SimBanner what="These experiment P&Ls"
                 note="Use them to rank models against each other, not to measure profit." />
      <div style={S.title}>A/B Experiments{data.version ? <span style={S.ver}> {data.version}</span> : null}</div>
      <div style={S.sub}>
        All models run in parallel on the same live prices (paper only).
        {data.cost_model_pct_round_trip != null && (
          <> Shadow trades charge a modeled {data.cost_model_pct_round_trip}% round-trip cost at exit.</>
        )}
        {data.config_frozen_until && (
          <span style={{ color: '#f59e0b' }}> Config frozen until {data.config_frozen_until} for clean data.</span>
        )}
      </div>


      <div style={{ ...S.title, fontSize: 15, marginTop: 18, marginBottom: 8 }}>Scoreboard — all models</div>
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
                  {/* This row read the simulator's own P&L while claiming to be
                      "REAL" — +$185.71 against a -$32.45 account. Now it comes
                      from Alpaca, and the simulator's number is shown beside it
                      so the gap is visible rather than quietly corrected. */}
                  {isReal && m.simulator_realized_pnl != null
                    && Math.abs((m.simulator_realized_pnl ?? 0) - (m.realized_pnl ?? 0)) >= 0.01 && (
                    <div style={S.divergence}>
                      simulator claims {(m.simulator_realized_pnl ?? 0) >= 0 ? '+' : ''}
                      ${(m.simulator_realized_pnl ?? 0).toFixed(2)} — overstated by $
                      {((m.simulator_realized_pnl ?? 0) - (m.realized_pnl ?? 0)).toFixed(2)}
                    </div>
                  )}
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
        <br /><br />
        REAL_TRADER is measured at the broker; the shadow models are simulated on the
        same prices. Compare the shadows against <b>each other</b> — that is the
        apples-to-apples test of whether the signals add anything.
      </div>
    </div>
  );
}

const E = {
  card: { background: '#0d1320', border: '1px solid #1e293b', borderRadius: 10, padding: 16, marginBottom: 8 },
  tinyLabel: { fontSize: '0.6rem', color: '#475569', textTransform: 'uppercase', letterSpacing: '0.05em', fontWeight: 700, marginBottom: 4 },
  tile: { textAlign: 'center', padding: '5px 0', background: '#111827', borderRadius: 4 },
  tileLabel: { fontSize: '0.5rem', color: '#475569', textTransform: 'uppercase' },
  tileValue: { fontSize: '0.8rem', fontWeight: 700 },
  table: { width: '100%', borderCollapse: 'collapse' },
  th: { textAlign: 'left', padding: '5px 8px', fontSize: 10, color: '#64748b', textTransform: 'uppercase', borderBottom: '1px solid #1e293b' },
  td: { padding: '5px 8px', fontSize: 12, color: '#e2e8f0', borderBottom: '1px solid #16202f' },
  empty: { fontSize: 12, color: '#64748b', padding: '8px 0' },
};

const S = {
  wrap: { padding: 20, maxWidth: 1080, margin: '0 auto' },
  title: { fontSize: 22, fontWeight: 700, color: '#f3f4f6' },
  ver: { fontSize: 11, fontWeight: 800, color: '#60a5fa', verticalAlign: 'middle', border: '1px solid #1d4ed8', borderRadius: 4, padding: '2px 6px', marginLeft: 8 },
  killBox: { fontSize: 11.5, color: '#fbbf24', background: 'rgba(245,158,11,0.08)', border: '1px solid rgba(245,158,11,0.3)', borderRadius: 8, padding: '8px 12px', marginBottom: 12, lineHeight: 1.5 },
  sub: { fontSize: 13, color: '#9ca3af', margin: '6px 0 14px 0', lineHeight: 1.5 },
  table: { width: '100%', borderCollapse: 'collapse', background: '#111827', borderRadius: 10, overflow: 'hidden' },
  th: { textAlign: 'left', padding: '10px 12px', fontSize: 11, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.5, borderBottom: '1px solid #374151' },
  td: { padding: '10px 12px', fontSize: 13, color: '#e5e7eb', borderBottom: '1px solid #1f2937' },
  tagExp: { fontSize: 9, fontWeight: 800, color: '#60a5fa', marginLeft: 6 },
  tagReal: { fontSize: 9, fontWeight: 800, color: '#34d399', marginLeft: 6 },
  divergence: { fontSize: 10.5, fontWeight: 600, color: '#fbbf24', marginTop: 3, lineHeight: 1.4 },
  note: { fontSize: 12, color: '#6b7280', lineHeight: 1.5, background: '#111827', padding: 12, borderRadius: 8, border: '1px solid #1f2937', marginTop: 14 },
  muted: { color: '#9ca3af', padding: 40, textAlign: 'center' },
  err: { color: '#dc2626', padding: 40, textAlign: 'center' },
};

export default ExperimentsPanel;
