import React, { useState, useEffect } from 'react';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

// Low-touch monthly ETF momentum rotation — the primary strategy.
function MomentumPanel() {
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const res = await fetch(`${API}/momentum`);
        const json = await res.json();
        if (active) { setData(json); setError(null); }
      } catch (e) {
        if (active) setError('Backend not reachable');
      }
    };
    load();
    const id = setInterval(load, 30000); // refresh every 30s (data changes monthly)
    return () => { active = false; clearInterval(id); };
  }, []);

  if (error) return <div style={S.wrap}><div style={S.err}>{error}</div></div>;
  if (!data) return <div style={S.wrap}><div style={S.muted}>Loading momentum portfolio…</div></div>;

  const beating = data.beating_benchmark;
  const edge = data.edge_vs_benchmark_pct ?? 0;
  const rollEdge = data.rolling_avg_edge_pct ?? 0;
  const col = (v) => (v >= 0 ? '#16a34a' : '#dc2626');

  return (
    <div style={S.wrap}>
      <div style={S.headerRow}>
        <div>
          <div style={S.title}>Momentum Portfolio</div>
          <div style={S.sub}>{data.strategy}</div>
          <div style={S.sub2}>{data.cadence} · benchmark {data.benchmark} · started {data.started_date || '—'}</div>
        </div>
        <div style={{ ...S.badge, background: beating ? '#dcfce7' : '#fee2e2', color: beating ? '#166534' : '#991b1b' }}>
          {beating ? 'BEATING ' + data.benchmark : 'BEHIND ' + data.benchmark}
        </div>
      </div>

      <div style={S.statRow}>
        <Stat label="Portfolio" value={`$${(data.portfolio_value ?? 0).toFixed(2)}`}
              sub={`${(data.portfolio_return_pct ?? 0) >= 0 ? '+' : ''}${(data.portfolio_return_pct ?? 0).toFixed(2)}%`}
              subColor={col(data.portfolio_return_pct ?? 0)} />
        <Stat label={`${data.benchmark} buy-hold`} value={`$${(data.benchmark_value ?? 0).toFixed(2)}`}
              sub={`${(data.benchmark_return_pct ?? 0) >= 0 ? '+' : ''}${(data.benchmark_return_pct ?? 0).toFixed(2)}%`}
              subColor={col(data.benchmark_return_pct ?? 0)} />
        <Stat label="Edge vs benchmark" value={`${edge >= 0 ? '+' : ''}${edge.toFixed(2)}%`} valueColor={col(edge)}
              sub={`rolling avg ${rollEdge >= 0 ? '+' : ''}${rollEdge.toFixed(2)}%`} subColor={col(rollEdge)} />
        <Stat label="Rebalances" value={`${data.rebalances ?? 0}`} sub={`last: ${data.last_rebalance_month || '—'}`} />
      </div>

      <div style={S.holdTitle}>Current Holdings (equal-weight)</div>
      <div style={S.holdGrid}>
        {(data.holdings || []).map((h, i) => {
          const m = /^(\S+)\s*\(([-+][\d.]+)%\)/.exec(h);
          const sym = m ? m[1] : h;
          const mom = m ? parseFloat(m[2]) : null;
          const isCash = sym === 'BIL' || sym === 'SHY';
          return (
            <div key={i} style={{ ...S.holdCard, borderColor: isCash ? '#f59e0b' : '#3b82f6' }}>
              <div style={S.holdSym}>{sym}{isCash && <span style={S.cashTag}> CASH</span>}</div>
              {mom !== null && <div style={{ ...S.holdMom, color: col(mom) }}>{mom >= 0 ? '+' : ''}{mom.toFixed(1)}% mom</div>}
            </div>
          );
        })}
      </div>

      <div style={S.note}>
        Ranked by rolling-average momentum (1/3/6-month blend). Rebalances monthly — check every few weeks, not daily.
        Slots with negative momentum rotate to BIL (T-bills) automatically for downturn protection.
      </div>
    </div>
  );
}

function Stat({ label, value, valueColor, sub, subColor }) {
  return (
    <div style={S.stat}>
      <div style={S.statLabel}>{label}</div>
      <div style={{ ...S.statValue, color: valueColor || '#e5e7eb' }}>{value}</div>
      {sub && <div style={{ ...S.statSub, color: subColor || '#9ca3af' }}>{sub}</div>}
    </div>
  );
}

const S = {
  wrap: { padding: 20, maxWidth: 960, margin: '0 auto' },
  headerRow: { display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 20 },
  title: { fontSize: 22, fontWeight: 700, color: '#f3f4f6' },
  sub: { fontSize: 13, color: '#9ca3af', marginTop: 4 },
  sub2: { fontSize: 12, color: '#6b7280', marginTop: 2 },
  badge: { padding: '6px 14px', borderRadius: 8, fontSize: 12, fontWeight: 700 },
  statRow: { display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 12, marginBottom: 24 },
  stat: { background: '#1f2937', border: '1px solid #374151', borderRadius: 10, padding: 14 },
  statLabel: { fontSize: 11, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.5 },
  statValue: { fontSize: 20, fontWeight: 700, marginTop: 6 },
  statSub: { fontSize: 13, fontWeight: 600, marginTop: 2 },
  holdTitle: { fontSize: 13, fontWeight: 700, color: '#9ca3af', textTransform: 'uppercase', letterSpacing: 0.5, marginBottom: 10 },
  holdGrid: { display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 10, marginBottom: 20 },
  holdCard: { background: '#111827', border: '2px solid #3b82f6', borderRadius: 10, padding: '14px 10px', textAlign: 'center' },
  holdSym: { fontSize: 16, fontWeight: 700, color: '#f3f4f6' },
  cashTag: { fontSize: 9, color: '#f59e0b', fontWeight: 700 },
  holdMom: { fontSize: 12, fontWeight: 600, marginTop: 4 },
  note: { fontSize: 12, color: '#6b7280', lineHeight: 1.5, background: '#111827', padding: 12, borderRadius: 8, border: '1px solid #1f2937' },
  muted: { color: '#9ca3af', padding: 40, textAlign: 'center' },
  err: { color: '#dc2626', padding: 40, textAlign: 'center' },
};

export default MomentumPanel;
