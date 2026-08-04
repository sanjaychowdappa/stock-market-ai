import React from 'react';

/**
 * Marks any panel whose numbers come from the internal simulator rather than
 * real Alpaca fills.
 *
 * These panels stay because they are useful diagnostics — relative comparisons
 * between models, signal behaviour, position state. What they are NOT is a
 * record of money made. The simulator's P&L has been wrong three separate ways,
 * and on days a real broker could check it, it reported gains on losing days.
 * So it gets to inform decisions, never to score them.
 */
function SimBanner({ what = 'These numbers', note }) {
  return (
    <div style={S.box}>
      <span style={S.tag}>SIMULATED</span>
      <span style={S.text}>
        {what} come from the internal simulator, <b>not from real broker fills</b> —
        no spread, no slippage, no rejections. Not a record of money made.
        {note ? ` ${note}` : ''} For actual results see the{' '}
        <b>Real P&amp;L (Alpaca)</b> tab.
      </span>
    </div>
  );
}

const S = {
  box: {
    display: 'flex', alignItems: 'baseline', gap: 10,
    background: 'rgba(245,158,11,0.07)', borderLeft: '3px solid #f59e0b',
    borderRadius: 8, padding: '9px 13px', marginBottom: 14,
  },
  tag: {
    fontSize: 9.5, fontWeight: 800, color: '#fbbf24', letterSpacing: 0.6,
    border: '1px solid #f59e0b66', borderRadius: 4, padding: '2px 6px',
    whiteSpace: 'nowrap', flexShrink: 0,
  },
  text: { fontSize: 12, color: '#cbd5e1', lineHeight: 1.55 },
};

export default SimBanner;
