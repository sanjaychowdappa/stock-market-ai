import React, { useState, useEffect } from 'react';

// NOTE: 127.0.0.1, not "localhost". On this machine localhost resolves to the
// IPv6 loopback (::1) where a wslrelay listener swallows port 8000. Port 3000
// happens to work through the same path; 8000 does not. Forcing IPv4 fixes it.
const API = process.env.REACT_APP_API_URL || 'http://127.0.0.1:8000/api';

/**
 * The REAL scoreboard.
 *
 * Every number on this panel comes from actual Alpaca paper fills — real
 * prices, real slippage, real rejections. The internal simulator is a
 * decision engine, not an accountant: its ledger has been wrong three
 * separate ways (cumulative-as-daily, mark-to-market inflation, duplicate
 * rows), and on the only days a broker could check it, it reported profits
 * on days that actually lost money. So it is not a source here.
 */
function BrokerPanel() {
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);

  const load = async () => {
    try {
      const d = await fetch(`${API}/broker`).then((r) => r.json());
      setData(d);
      setError(null);
    } catch (e) {
      setError('Backend not reachable');
    }
  };

  useEffect(() => {
    load();
    const id = setInterval(load, 15000);
    return () => clearInterval(id);
  }, []);

  if (error) return <div style={S.wrap}><div style={S.err}>{error}</div></div>;
  if (!data) return <div style={S.wrap}><div style={S.muted}>Loading real broker data…</div></div>;

  const r = data.real_pnl || {};
  const f = data.fills || {};
  const acct = data.account || {};

  // Always show the sign — a loss must never render as if it were a gain.
  const money = (v) => `${v >= 0 ? '+' : '-'}$${Math.abs(Number(v) || 0).toFixed(2)}`;
  const col = (v) => (v >= 0 ? '#22c55e' : '#ef4444');

  const real = Number(r.real_realized_pnl) || 0;
  const sim = Number(r.simulator_would_have_shown) || 0;
  const drag = Number(r.execution_drag) || 0;
  const trips = r.round_trips || 0;

  const recent = f.recent || [];
  const dq = r.data_quality || {};

  return (
    <div style={S.wrap}>
      <div style={S.headRow}>
        <div>
          <div style={S.title}>
            Real P&amp;L <span style={S.ver}>ALPACA PAPER</span>
          </div>
          <div style={S.sub}>
            Actual broker fills — real prices, real slippage, real rejections.
            This is the only number that counts.
          </div>
        </div>
        <div style={{ textAlign: 'right' }}>
          <div style={{
            ...S.badge,
            background: data.connected ? 'rgba(34,197,94,0.12)' : 'rgba(239,68,68,0.12)',
            color: data.connected ? '#22c55e' : '#ef4444',
            border: `1px solid ${data.connected ? '#22c55e' : '#ef4444'}55`,
          }}>
            {data.connected ? `CONNECTED · ${acct.status || ''}` : 'DISCONNECTED'}
          </div>
          <div style={S.tiny}>equity ${Number(acct.equity || 0).toLocaleString()}</div>
        </div>
      </div>

      {/* ── The headline ────────────────────────────────────── */}
      <div style={{ ...S.hero, borderColor: `${col(real)}44` }}>
        <div style={S.heroLabel}>REALIZED P&amp;L — ACTUAL FILLS</div>
        <div style={{ ...S.heroValue, color: col(real) }}>{money(real)}</div>
        <div style={S.heroSub}>
          {trips} completed round trip{trips === 1 ? '' : 's'} ·{' '}
          {r.wins || 0}W / {r.losses || 0}L · {(r.win_rate_pct || 0).toFixed(1)}% win rate
        </div>
      </div>

      <div style={S.statRow}>
        <Stat label="Per round trip" value={money(r.avg_per_round_trip || 0)}
              color={col(r.avg_per_round_trip || 0)}
              sub="average, realized" />
        <Stat label="Execution drag" value={money(-Math.abs(drag))} color="#ef4444"
              sub="cost the simulator never charged" />
        <Stat label="Slippage paid" value={money(-Math.abs(f.total_slippage_cost || 0))}
              color="#ef4444"
              sub={`avg ${(f.avg_slippage_pct || 0).toFixed(3)}% per fill`} />
        <Stat label="Order quality" value={`${f.filled || 0} filled`} color="#93c5fd"
              sub={`${f.rejected || 0} rejected · ${f.unfilled || 0} unfilled`} />
      </div>

      {/* ── Simulator contrast — explicitly NOT the score ───── */}
      <div style={S.warnBox}>
        <div style={S.warnTitle}>SIMULATOR CLAIMED — NOT THE SCOREBOARD</div>
        <div style={S.warnBody}>
          On these same trades the internal simulator reported{' '}
          <b style={{ color: col(sim) }}>{money(sim)}</b>, against a real result of{' '}
          <b style={{ color: col(real) }}>{money(real)}</b>. The{' '}
          <b>{money(-Math.abs(drag))}</b> gap is execution cost the simulator does not model.
          Its daily ledger is separately unreliable — duplicated dates, and profits
          booked on days that really lost money — so it is not reported anywhere on
          this dashboard.
        </div>
      </div>

      {(dq.partial_qty_rows > 0 || dq.unmatched_sells > 0) && (
        <div style={S.warnBox}>
          <div style={S.warnTitle}>HISTORICAL FILL QUANTITIES ARE APPROXIMATE</div>
          <div style={S.warnBody}>
            <b>{dq.partial_qty_rows} fill(s)</b> were recorded before the partial-fill
            parse fix: the order poller stopped at the first price it saw, which Alpaca
            also reports while an order is still <i>partially filled</i>, so a
            mid-execution snapshot was stored as the final quantity.
            {dq.unmatched_sells > 0 && (
              <> It also left <b>{dq.unmatched_sells} sell(s)</b> ({Number(dq.unmatched_qty).toFixed(3)} sh)
              without a matching buy lot.</>
            )}{' '}
            The P&amp;L above is therefore <b>approximate for those days</b>. Fills
            recorded from 2026-08-05 onward wait for a terminal order status and are exact.
          </div>
        </div>
      )}

      {/* ── Daily breakdown ────────────────────────────────── */}
      <div style={S.sectionLabel}>REAL P&amp;L BY DAY</div>
      <div style={S.box}>
        <table style={S.table}>
          <tbody>
            {(r.by_day || []).slice().reverse().map((d) => (
              <tr key={d.date}>
                <td style={S.td}>{d.date}</td>
                <td style={{ ...S.td, color: col(d.real_pnl), fontWeight: 700, textAlign: 'right' }}>
                  {money(d.real_pnl)}
                </td>
              </tr>
            ))}
            {(!r.by_day || r.by_day.length === 0) && (
              <tr><td style={{ ...S.td, color: '#64748b' }}>No completed round trips yet.</td></tr>
            )}
          </tbody>
        </table>
      </div>

      {/* ── Recent orders ──────────────────────────────────── */}
      <div style={S.sectionLabel}>RECENT BROKER ORDERS</div>
      <div style={S.box}>
        <table style={S.table}>
          <thead>
            <tr>
              {['Time', 'Symbol', 'Side', 'Qty', 'Fill', 'Slip', 'Outcome', 'Reason'].map((h) => (
                <th key={h} style={S.th}>{h}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {recent.slice(0, 15).map((o) => {
              const oc = o.outcome === 'filled' ? '#22c55e'
                : o.outcome === 'rejected' ? '#ef4444' : '#f59e0b';
              return (
                <tr key={o.order_id || o.timestamp}>
                  <td style={S.td}>{new Date(o.timestamp).toLocaleTimeString()}</td>
                  <td style={{ ...S.td, fontWeight: 700 }}>{o.symbol}</td>
                  <td style={{ ...S.td, color: o.side === 'buy' ? '#60a5fa' : '#f472b6' }}>
                    {o.side.toUpperCase()}
                  </td>
                  <td style={S.td}>{Number(o.qty_requested ?? o.qty ?? 0).toFixed(3)}</td>
                  <td style={S.td}>
                    {o.actual_price ? `$${Number(o.actual_price).toFixed(2)}` : '—'}
                  </td>
                  <td style={{ ...S.td, color: (o.slippage_pct || 0) > 0 ? '#ef4444' : '#64748b' }}>
                    {o.slippage_pct != null ? `${o.slippage_pct.toFixed(3)}%` : '—'}
                  </td>
                  <td style={{ ...S.td, color: oc, fontWeight: 700 }}>{o.outcome}</td>
                  <td style={{ ...S.td, color: '#64748b', fontSize: 10.5 }}>{o.reason}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <div style={S.boundary}>
        <b>Why Alpaca only:</b> a paper broker charges real spread, real slippage and
        really rejects orders; a simulator charges whatever it was told to. If a
        strategy cannot profit here, it cannot profit with real money — so the
        simulator's P&amp;L is deliberately not shown as a result anywhere.
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
  headRow: { display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 14 },
  title: { fontSize: 22, fontWeight: 700, color: '#f3f4f6' },
  ver: { fontSize: 11, fontWeight: 800, color: '#60a5fa', border: '1px solid #1d4ed8', borderRadius: 4, padding: '2px 6px', marginLeft: 8, verticalAlign: 'middle' },
  sub: { fontSize: 12.5, color: '#9ca3af', marginTop: 4, maxWidth: 560, lineHeight: 1.5 },
  badge: { display: 'inline-block', padding: '5px 14px', borderRadius: 8, fontSize: 12, fontWeight: 800 },
  tiny: { fontSize: 10.5, color: '#64748b', marginTop: 5 },
  hero: { background: '#0d1320', border: '1px solid', borderRadius: 12, padding: '18px 20px', marginBottom: 12 },
  heroLabel: { fontSize: 10.5, fontWeight: 800, color: '#475569', letterSpacing: 0.7 },
  heroValue: { fontSize: 42, fontWeight: 800, lineHeight: 1.15, marginTop: 4 },
  heroSub: { fontSize: 12.5, color: '#94a3b8', marginTop: 4 },
  statRow: { display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 },
  stat: { background: '#111827', border: '1px solid #1f2937', borderRadius: 10, padding: 12 },
  statLabel: { fontSize: 10, color: '#64748b', textTransform: 'uppercase', letterSpacing: 0.4 },
  statValue: { fontSize: 19, fontWeight: 800, marginTop: 5 },
  statSub: { fontSize: 10.5, color: '#64748b', marginTop: 3 },
  sectionLabel: { fontSize: 11, fontWeight: 800, color: '#475569', textTransform: 'uppercase', letterSpacing: 0.6, margin: '18px 0 8px 0' },
  box: { background: '#0d1320', border: '1px solid #1e293b', borderRadius: 10, padding: 12, overflowX: 'auto' },
  table: { width: '100%', borderCollapse: 'collapse' },
  th: { padding: '4px 6px', fontSize: 9.5, color: '#475569', textTransform: 'uppercase', letterSpacing: 0.4, textAlign: 'left', borderBottom: '1px solid #1e293b' },
  td: { padding: '5px 6px', fontSize: 12, color: '#e2e8f0', borderBottom: '1px solid #16202f', whiteSpace: 'nowrap' },
  warnBox: { background: 'rgba(245,158,11,0.07)', borderLeft: '3px solid #f59e0b', borderRadius: 8, padding: '11px 14px', marginTop: 12 },
  warnTitle: { fontSize: 10.5, fontWeight: 800, color: '#fbbf24', letterSpacing: 0.5 },
  warnBody: { fontSize: 12.5, color: '#cbd5e1', marginTop: 5, lineHeight: 1.55 },
  boundary: { fontSize: 11.5, color: '#6b7280', background: '#111827', border: '1px solid #1f2937', borderRadius: 8, padding: 12, marginTop: 16, lineHeight: 1.55 },
  muted: { color: '#9ca3af', padding: 40, textAlign: 'center' },
  err: { color: '#dc2626', padding: 40, textAlign: 'center' },
};

export default BrokerPanel;
