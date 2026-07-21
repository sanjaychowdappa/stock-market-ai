import React, { useState, useEffect } from 'react';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

const fmtHold = (s) => {
  if (s == null) return '';
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${s % 60 > 0 ? ` ${s % 60}s` : ''}`;
};

/* ─── exp1 live panel (legacy-trader style) ─────────────────────── */
function Exp1Live() {
  const [d, setD] = useState(null);

  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const res = await fetch(`${API}/exp1`);
        const json = await res.json();
        if (active) setD(json);
      } catch (e) { /* backend down — scoreboard shows its own error */ }
    };
    load();
    const id = setInterval(load, 5000);
    return () => { active = false; clearInterval(id); };
  }, []);

  if (!d || d.error) return null;
  const up = (d.realized_pnl ?? 0) >= 0;
  const clr = up ? '#22c55e' : '#ef4444';

  return (
    <div style={E.card}>
      {/* Header row — like the legacy portfolio header */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div>
          <div style={E.tinyLabel}>EXP1 — LIVE (short-horizon prediction)</div>
          <div style={{ fontSize: '1.5rem', fontWeight: 800, color: '#e2e8f0' }}>
            ${(d.portfolio_value ?? 0).toFixed(2)}
          </div>
          <div style={{ fontSize: '0.85rem', fontWeight: 700, color: clr }}>
            {up ? '+' : ''}{(d.realized_pnl ?? 0).toFixed(2)} realized
          </div>
        </div>
        <div style={{ textAlign: 'right', fontSize: '0.7rem', color: '#94a3b8' }}>
          {d.total_trades} trades | {(d.win_rate_pct ?? 0).toFixed(0)}% win
          <div style={{ marginTop: 4, color: '#475569', fontSize: '0.6rem', maxWidth: 340, lineHeight: 1.4 }}>
            {d.strategy}
          </div>
        </div>
      </div>

      {/* Cash / Invested / Realized tiles — like legacy */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 6, margin: '8px 0' }}>
        {[
          { label: 'Cash', value: `$${(d.cash ?? 0).toFixed(2)}`, color: '#94a3b8' },
          { label: 'Invested', value: `$${(d.invested ?? 0).toFixed(2)}`, color: '#3b82f6' },
          { label: 'Realized', value: `${up ? '+' : ''}$${(d.realized_pnl ?? 0).toFixed(2)}`, color: clr },
        ].map((item, i) => (
          <div key={i} style={E.tile}>
            <div style={E.tileLabel}>{item.label}</div>
            <div style={{ ...E.tileValue, color: item.color }}>{item.value}</div>
          </div>
        ))}
      </div>

      {/* Open positions */}
      <div style={E.tinyLabel}>OPEN POSITIONS ({(d.positions || []).length})</div>
      {(d.positions || []).length === 0 ? (
        <div style={E.empty}>Flat — waiting for a next-minute forecast &gt; +0.08%</div>
      ) : (
        <table style={E.table}>
          <thead><tr>{['Sym', 'Shares', 'Entry', 'Now', 'P&L', 'P&L %', 'Held'].map(h => <th key={h} style={E.th}>{h}</th>)}</tr></thead>
          <tbody>
            {d.positions.map((p) => (
              <tr key={p.symbol}>
                <td style={{ ...E.td, fontWeight: 700 }}>{p.symbol}</td>
                <td style={E.td}>{p.shares}</td>
                <td style={E.td}>${p.entry_price}</td>
                <td style={E.td}>${p.current_price}</td>
                <td style={{ ...E.td, color: p.pnl >= 0 ? '#22c55e' : '#ef4444' }}>{p.pnl >= 0 ? '+' : ''}{p.pnl}</td>
                <td style={{ ...E.td, color: p.pnl >= 0 ? '#22c55e' : '#ef4444' }}>{p.pnl_pct}%</td>
                <td style={E.td}>{fmtHold(p.hold_seconds)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {/* Trade log */}
      <div style={{ ...E.tinyLabel, marginTop: 10 }}>RECENT TRADES</div>
      {(d.recent_trades || []).length === 0 ? (
        <div style={E.empty}>No trades yet today.</div>
      ) : (
        <div style={{ maxHeight: 220, overflowY: 'auto' }}>
          <table style={E.table}>
            <tbody>
              {d.recent_trades.map((t, i) => (
                <tr key={i}>
                  <td style={{ ...E.td, color: '#64748b' }}>{t.time}</td>
                  <td style={{ ...E.td, fontWeight: 800, color: t.action === 'BUY' ? '#3b82f6' : (t.pnl ?? 0) >= 0 ? '#22c55e' : '#ef4444' }}>{t.action}</td>
                  <td style={{ ...E.td, fontWeight: 700 }}>{t.symbol}</td>
                  <td style={E.td}>${t.price}</td>
                  <td style={{ ...E.td, color: (t.pnl ?? 0) >= 0 ? '#22c55e' : '#ef4444' }}>
                    {t.pnl != null ? `${t.pnl >= 0 ? '+' : ''}${t.pnl.toFixed(2)} (${t.pnl_pct?.toFixed(2)}%)` : ''}
                  </td>
                  <td style={{ ...E.td, color: '#64748b', fontSize: 10.5 }}>{t.reason}{t.hold_seconds != null ? ` · ${fmtHold(t.hold_seconds)}` : ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

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
      <div style={S.title}>A/B Experiments</div>
      <div style={S.sub}>
        All models run in parallel on the same live prices (paper only).
      </div>

      <Exp1Live />

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
  sub: { fontSize: 13, color: '#9ca3af', margin: '6px 0 14px 0', lineHeight: 1.5 },
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
