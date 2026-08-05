import React, { useState, useEffect } from 'react';

const API = process.env.REACT_APP_API_URL || 'http://127.0.0.1:8000/api';

/**
 * App-wide banner shown whenever damage control has real orders halted.
 *
 * During a halt the simulator keeps trading, so every position panel in the app
 * fills up with holdings while the broker account is flat. Seeing five open
 * positions during what is supposed to be a stop is alarming and reads as a
 * failure — the distinction has to be visible without opening a tab or reading
 * a footnote, so it lives above the tab bar rather than inside a panel.
 */
function HaltBanner() {
  const [dc, setDc] = useState(null);

  useEffect(() => {
    const load = () => {
      fetch(`${API}/damage-control`)
        .then((r) => r.json())
        .then(setDc)
        .catch(() => {});
    };
    load();
    const id = setInterval(load, 10000);
    return () => clearInterval(id);
  }, []);

  if (!dc || !dc.enabled || !dc.halted) return null;

  const trades = dc.recovery_trades ?? 0;
  const needed = dc.recovery_trades_needed ?? 3;
  const pnl = Number(dc.recovery_pnl ?? 0);
  const pnlOk = pnl > (dc.recovery_pnl_needed ?? 0);

  return (
    <div style={S.wrap}>
      <div style={S.row}>
        <span style={S.tag}>REAL ORDERS HALTED</span>
        <span style={S.text}>
          <b>No money is at risk.</b> Alpaca is flat. Any positions shown anywhere in this
          app are the <b>simulator</b>, which keeps trading to earn its way back — that is
          the design, not a leak.
        </span>
      </div>
      <div style={S.progress}>
        <span style={S.pLabel}>Recovery gate</span>
        <span style={{ color: trades >= needed ? '#22c55e' : '#fbbf24', fontWeight: 800 }}>
          {trades}/{needed} closed trades
        </span>
        <span style={{ color: '#475569' }}>·</span>
        <span style={{ color: pnlOk ? '#22c55e' : '#f87171', fontWeight: 800 }}>
          {pnl >= 0 ? '+' : '-'}${Math.abs(pnl).toFixed(2)} net of costs
        </span>
        <span style={S.pNote}>
          Alpaca re-engages when both clear. Day P&amp;L {dc.day_pnl_pct?.toFixed(2)}% ·
          floor {dc.floor_pct?.toFixed(2)}%
        </span>
      </div>
    </div>
  );
}

const S = {
  wrap: {
    background: 'rgba(239,68,68,0.10)',
    borderTop: '2px solid #ef4444',
    borderBottom: '1px solid rgba(239,68,68,0.35)',
    padding: '10px 18px',
  },
  row: { display: 'flex', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' },
  tag: {
    fontSize: 10.5, fontWeight: 800, color: '#fff', background: '#dc2626',
    padding: '3px 9px', borderRadius: 5, letterSpacing: 0.6, whiteSpace: 'nowrap',
  },
  text: { fontSize: 12.5, color: '#e2e8f0', lineHeight: 1.55 },
  progress: {
    display: 'flex', alignItems: 'baseline', gap: 9, flexWrap: 'wrap',
    marginTop: 7, fontSize: 12,
  },
  pLabel: { fontSize: 10, fontWeight: 800, color: '#94a3b8', letterSpacing: 0.5, textTransform: 'uppercase' },
  pNote: { fontSize: 11, color: '#64748b' },
};

export default HaltBanner;
