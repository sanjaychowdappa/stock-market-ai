import React, { useState, useEffect } from 'react';

const API = process.env.REACT_APP_API_URL || 'http://127.0.0.1:8000/api';

const SEV = {
  critical: { bg: 'rgba(220,38,38,0.12)', bd: '#dc2626', fg: '#f87171', icon: '!' },
  warn: { bg: 'rgba(245,158,11,0.10)', bd: '#f59e0b', fg: '#fbbf24', icon: '!' },
  info: { bg: 'rgba(59,130,246,0.07)', bd: '#3b82f6', fg: '#93c5fd', icon: 'i' },
};

const HEALTH = {
  healthy: { fg: '#22c55e', bg: 'rgba(34,197,94,0.12)', label: 'HEALTHY' },
  degraded: { fg: '#f59e0b', bg: 'rgba(245,158,11,0.12)', label: 'DEGRADED' },
  critical: { fg: '#ef4444', bg: 'rgba(239,68,68,0.12)', label: 'CRITICAL' },
};

function AgenticPanel() {
  const [agent, setAgent] = useState(null);
  const [broker, setBroker] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);

  const load = async () => {
    try {
      // P&L comes from the broker, never from /api/profit. That ledger has
      // duplicated dates and books gains on days that really lost money.
      const [a, b] = await Promise.all([
        fetch(`${API}/agentic`).then(r => r.json()),
        fetch(`${API}/broker`).then(r => r.json()).catch(() => null),
      ]);
      setAgent(a); setBroker(b); setError(null);
    } catch (err) { setError('Backend not reachable'); }
  };

  useEffect(() => {
    load();
    const id = setInterval(load, 15000);
    return () => clearInterval(id);
  }, []);

  const runNow = async () => {
    setBusy(true);
    try {
      const a = await fetch(`${API}/agentic/run`).then(r => r.json());
      setAgent(a);
    } catch (e) { /* ignore */ }
    setBusy(false);
  };

  if (error) return <div style={S.wrap}><div style={S.err}>{error}</div></div>;
  if (!agent) return <div style={S.wrap}><div style={S.muted}>Loading agentic module…</div></div>;

  const h = HEALTH[agent.health] || HEALTH.healthy;
  // Always show the sign — a loss must never render as if it were a gain.
  const money = (v) => `${v >= 0 ? '+' : '-'}$${Math.abs(v).toFixed(2)}`;
  const col = (v) => (v >= 0 ? '#22c55e' : '#ef4444');

  const rp = broker?.real_pnl || {};
  const realized = Number(rp.real_realized_pnl) || 0;
  const days = rp.by_day || [];
  const today = days.length ? days[days.length - 1] : null;

  return (
    <div style={S.wrap}>
      {/* ── Header ─────────────────────────────────────────── */}
      <div style={S.headRow}>
        <div>
          <div style={S.title}>
            agentic_test <span style={S.ver}>{agent.version}</span>
          </div>
          <div style={S.sub}>{agent.role}</div>
        </div>
        <div style={{ textAlign: 'right' }}>
          <div style={{ ...S.badge, background: h.bg, color: h.fg, border: `1px solid ${h.fg}55` }}>{h.label}</div>
          <div style={S.tiny}>{agent.runs} cycles · last {agent.last_run ? new Date(agent.last_run).toLocaleTimeString() : '—'}</div>
          <button onClick={runNow} disabled={busy} style={S.btn}>{busy ? 'Running…' : 'Run check now'}</button>
        </div>
      </div>

      <div style={S.summary}>{agent.summary}</div>

      {/* ── P&L it oversees — real broker fills only ───────── */}
      <div style={S.sectionLabel}>PROFITS &amp; GAINS UNDER SUPERVISION — ALPACA REAL FILLS</div>
      <div style={S.statRow}>
        <Stat label="Realized P&L (real)" value={money(realized)} color={col(realized)}
              sub={`${rp.round_trips ?? 0} round trip(s) · ${(rp.win_rate_pct ?? 0).toFixed(0)}% win`} />
        <Stat label="Latest session" value={today ? money(today.real_pnl) : '—'}
              color={today ? col(today.real_pnl) : '#94a3b8'}
              sub={today ? today.date : 'no completed round trips'} />
        <Stat label="Per round trip" value={money(rp.avg_per_round_trip ?? 0)}
              color={col(rp.avg_per_round_trip ?? 0)} sub="average, after real costs" />
      </div>

      {days.length > 0 && (
        <div style={S.weekBox}>
          <div style={S.tinyLabel}>REAL P&amp;L BY DAY (ALPACA)</div>
          <table style={S.table}>
            <tbody>
              {days.slice().reverse().map((d) => (
                <tr key={d.date}>
                  <td style={S.td}>{d.date}</td>
                  <td style={{ ...S.td, color: col(d.real_pnl), fontWeight: 700 }}>{money(d.real_pnl)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* ── Agent activity ─────────────────────────────────── */}
      <div style={S.sectionLabel}>AGENT ACTIVITY — AUTOMATED CHECKS</div>
      {(agent.findings || []).map((f, i) => {
        const s = SEV[f.severity] || SEV.info;
        return (
          <div key={i} style={{ ...S.finding, background: s.bg, borderLeft: `3px solid ${s.bd}` }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
              <span style={{ ...S.checkName, color: s.fg }}>{f.check}</span>
              <span style={{ ...S.sevTag, color: s.fg }}>{f.severity.toUpperCase()}</span>
            </div>
            <div style={S.msg}>{f.message}</div>
            {f.recommendation && (
              <div style={S.rec}><b>Recommends:</b> {f.recommendation}</div>
            )}
          </div>
        );
      })}

      <div style={S.boundary}>
        <b>Safety boundary:</b> {agent.boundary}
      </div>
    </div>
  );
}

function Stat({ label, value, color, sub }) {
  return (
    <div style={S.stat}>
      <div style={S.statLabel}>{label}</div>
      <div style={{ ...S.statValue, color: color || '#e5e7eb' }}>{value}</div>
      {sub && <div style={S.statSub}>{sub}</div>}
    </div>
  );
}

const S = {
  wrap: { padding: 20, maxWidth: 1000, margin: '0 auto' },
  headRow: { display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 10 },
  title: { fontSize: 22, fontWeight: 700, color: '#f3f4f6' },
  ver: { fontSize: 11, fontWeight: 800, color: '#60a5fa', border: '1px solid #1d4ed8', borderRadius: 4, padding: '2px 6px', marginLeft: 8, verticalAlign: 'middle' },
  sub: { fontSize: 12.5, color: '#9ca3af', marginTop: 4, maxWidth: 560, lineHeight: 1.5 },
  badge: { display: 'inline-block', padding: '5px 14px', borderRadius: 8, fontSize: 12, fontWeight: 800 },
  tiny: { fontSize: 10.5, color: '#64748b', marginTop: 5 },
  btn: { display: 'block', marginTop: 8, marginLeft: 'auto', background: '#1e293b', color: '#93c5fd', border: '1px solid #334155', borderRadius: 6, padding: '5px 12px', fontSize: 11, cursor: 'pointer' },
  summary: { fontSize: 13, color: '#cbd5e1', background: '#111827', border: '1px solid #1f2937', borderRadius: 8, padding: '10px 14px', marginBottom: 16 },
  sectionLabel: { fontSize: 11, fontWeight: 800, color: '#475569', textTransform: 'uppercase', letterSpacing: 0.6, margin: '18px 0 8px 0' },
  statRow: { display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10 },
  stat: { background: '#111827', border: '1px solid #1f2937', borderRadius: 10, padding: 12 },
  statLabel: { fontSize: 10, color: '#64748b', textTransform: 'uppercase', letterSpacing: 0.4 },
  statValue: { fontSize: 19, fontWeight: 800, marginTop: 5 },
  statSub: { fontSize: 10.5, color: '#64748b', marginTop: 3 },
  weekBox: { background: '#0d1320', border: '1px solid #1e293b', borderRadius: 10, padding: 12, marginTop: 10 },
  tinyLabel: { fontSize: 10, fontWeight: 700, color: '#475569', textTransform: 'uppercase', marginBottom: 6 },
  table: { width: '100%', borderCollapse: 'collapse' },
  td: { padding: '4px 6px', fontSize: 12, color: '#e2e8f0', borderBottom: '1px solid #16202f' },
  finding: { borderRadius: 8, padding: '10px 14px', marginBottom: 8 },
  checkName: { fontSize: 12.5, fontWeight: 800, fontFamily: 'monospace' },
  sevTag: { fontSize: 9.5, fontWeight: 800, letterSpacing: 0.5 },
  msg: { fontSize: 12.5, color: '#cbd5e1', marginTop: 4, lineHeight: 1.5 },
  rec: { fontSize: 11.5, color: '#fbbf24', marginTop: 5, lineHeight: 1.45 },
  boundary: { fontSize: 11.5, color: '#6b7280', background: '#111827', border: '1px solid #1f2937', borderRadius: 8, padding: 12, marginTop: 16, lineHeight: 1.5 },
  muted: { color: '#9ca3af', padding: 40, textAlign: 'center' },
  err: { color: '#dc2626', padding: 40, textAlign: 'center' },
};

export default AgenticPanel;
