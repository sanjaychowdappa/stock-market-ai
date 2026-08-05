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
  const [dmg, setDmg] = useState(null);
  const [error, setError] = useState(null);

  const load = async () => {
    try {
      const [d, dc] = await Promise.all([
        fetch(`${API}/broker`).then((r) => r.json()),
        fetch(`${API}/damage-control`).then((r) => r.json()).catch(() => null),
      ]);
      setData(d);
      setDmg(dc);
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

  const eq = data.equity_pnl || {};
  const net = Number(eq.net_pnl) || 0;
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

      {/* ── The headline: Alpaca's own equity curve ─────────── */}
      {eq.available ? (
        <div style={{ ...S.hero, borderColor: `${col(net)}44` }}>
          <div style={S.heroLabel}>NET P&amp;L — ALPACA'S OWN BOOKS</div>
          <div style={{ ...S.heroValue, color: col(net) }}>{money(net)}</div>
          <div style={S.heroSub}>
            ${Number(eq.starting_equity).toLocaleString()} → $
            {Number(eq.current_equity).toLocaleString()} ({net >= 0 ? '+' : ''}
            {Number(eq.net_pnl_pct).toFixed(4)}%)
          </div>
        </div>
      ) : (
        <div style={{ ...S.hero, borderColor: '#f59e0b44' }}>
          <div style={S.heroLabel}>NET P&amp;L — ALPACA'S OWN BOOKS</div>
          <div style={{ ...S.heroValue, color: '#f59e0b' }}>unavailable</div>
          <div style={S.heroSub}>{eq.reason || 'could not reach the broker'}</div>
        </div>
      )}

      {/* ── Damage control ─────────────────────────────────── */}
      {dmg && dmg.enabled && (
        <div style={{
          ...S.dcBox,
          borderColor: dmg.halted ? '#ef4444' : dmg.headroom_pct < 0.3 ? '#f59e0b' : '#1e293b',
        }}>
          <div style={S.dcHead}>
            <span style={S.dcTitle}>DAMAGE CONTROL</span>
            <span style={{
              ...S.dcBadge,
              background: dmg.halted ? 'rgba(239,68,68,0.15)' : 'rgba(34,197,94,0.12)',
              color: dmg.halted ? '#ef4444' : '#22c55e',
            }}>
              {dmg.halted
                ? 'REAL ORDERS HALTED · simulator still trading'
                : 'LIVE · real orders active'}
            </span>
          </div>
          <div style={S.dcGrid}>
            <Mini label="Day P&L" value={`${dmg.day_pnl_pct >= 0 ? '+' : ''}${dmg.day_pnl_pct.toFixed(2)}%`}
                  color={col(dmg.day_pnl_pct)} />
            <Mini label="Floor" value={`${dmg.floor_pct.toFixed(2)}%`} color="#f87171"
                  sub={`$${Number(dmg.floor_value).toFixed(0)}`} />
            <Mini label="Headroom" value={`${dmg.headroom_pct.toFixed(2)}%`}
                  color={dmg.headroom_pct < 0.3 ? '#f59e0b' : '#93c5fd'}
                  sub="before halt" />
            <Mini label="Day peak" value={`${dmg.day_peak_pnl_pct >= 0 ? '+' : ''}${dmg.day_peak_pnl_pct.toFixed(2)}%`}
                  color={col(dmg.day_peak_pnl_pct)}
                  sub={dmg.profit_lock_armed ? 'lock ARMED' : `lock at +${dmg.profit_lock_trigger_pct}%`} />
            {/* The cap governs REAL orders. While halted none are placed, so it
                is suspended — saying "cap reached" there would misdescribe why
                the simulator is still opening positions. */}
            <Mini label="Entries" value={`${dmg.entries_today} / ${dmg.entry_cap}`}
                  color={dmg.halted ? '#64748b' : dmg.entries_today >= dmg.entry_cap ? '#f59e0b' : '#93c5fd'}
                  sub={dmg.halted
                    ? 'cap suspended (sim only)'
                    : dmg.entries_today >= dmg.entry_cap ? 'cap reached' : 'today'} />
          </div>
          {dmg.halted && (
            <div style={S.recBox}>
              <div style={S.recTitle}>RECOVERY GATE — earning its way back</div>
              <div style={S.recBody}>
                Real money is out of the market. The simulator keeps trading, and Alpaca
                re-engages once it has proven itself — <b>not</b> when a timer expires and
                <b> not</b> when the balance returns to $3,000.
              </div>
              <div style={S.recStats}>
                <span>
                  Closed trades:{' '}
                  <b style={{ color: dmg.recovery_trades >= dmg.recovery_trades_needed ? '#22c55e' : '#fbbf24' }}>
                    {dmg.recovery_trades} / {dmg.recovery_trades_needed}
                  </b>
                </span>
                <span>
                  Net of costs:{' '}
                  <b style={{ color: col(dmg.recovery_pnl) }}>{money(dmg.recovery_pnl)}</b>
                  {' '}<span style={{ color: '#64748b' }}>(need &gt; ${dmg.recovery_pnl_needed})</span>
                </span>
              </div>
            </div>
          )}
          <div style={S.dcNote}>
            Below the floor the book is flattened and <b>real orders stop</b> — the simulator
            trades on, because simulated trades cost nothing and are the only way to learn
            whether the model has recovered. Each recovery trade is charged a modeled
            round-trip cost first, so the gate measures the model rather than the simulator's
            optimism. Once the day peaks above +{dmg.profit_lock_trigger_pct}%, the floor
            ratchets up behind it so a winning day cannot become a losing one. This{' '}
            <b>bounds</b> a loss — it cannot eliminate one: a stop fills below its trigger
            and every exit pays a round trip.
          </div>
        </div>
      )}

      <div style={S.sectionLabel}>COST ATTRIBUTION — DIAGNOSTIC, NOT THE RESULT</div>
      <div style={S.statRow}>
        <Stat label="Per round trip" value={money(r.avg_per_round_trip || 0)}
              color={col(r.avg_per_round_trip || 0)}
              sub={`${trips} trips · ${(r.win_rate_pct || 0).toFixed(0)}% win · reconstructed`} />
        <Stat label="Execution drag" value={money(-Math.abs(drag))} color="#ef4444"
              sub="cost the simulator never charged" />
        <Stat label="Slippage paid" value={money(-Math.abs(f.total_slippage_cost || 0))}
              color="#ef4444"
              sub={`avg ${(f.avg_slippage_pct || 0).toFixed(3)}% per fill`} />
        {/* "unfilled" is OUR label, not Alpaca's. Every order the broker
            accepted has filled; the three we marked unfilled on 08-04 had all
            completed at their full requested size by the time we gave up
            polling. Rejections are real — they never became orders. */}
        <Stat label="Order quality (our log)" value={`${f.filled || 0} filled`} color="#93c5fd"
              sub={`${f.rejected || 0} rejected · ${f.unfilled || 0} logged unfilled (broker shows 0)`} />
      </div>

      {/* ── Simulator contrast — explicitly NOT the score ───── */}
      <div style={S.warnBox}>
        <div style={S.warnTitle}>SIMULATOR CLAIMED — NOT THE SCOREBOARD</div>
        <div style={S.warnBody}>
          Priced the simulator's way, these trades come to{' '}
          <b style={{ color: col(sim) }}>{money(sim)}</b>; priced at the fills we recorded,{' '}
          <b style={{ color: col(real) }}>{money(real)}</b>. The <b>{money(-Math.abs(drag))}</b>{' '}
          gap is execution cost the simulator does not model — that comparison is still
          useful. Neither figure is the result: the account is{' '}
          <b style={{ color: col(net) }}>{money(net)}</b> per Alpaca. The simulator's daily
          ledger is separately unreliable — it re-banked the same dollars three times — so
          it is not reported anywhere on this dashboard.
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

      {/* ── Daily breakdown, from Alpaca's own books ───────── */}
      <div style={S.sectionLabel}>NET P&amp;L BY DAY — ALPACA</div>
      <div style={S.box}>
        <table style={S.table}>
          <tbody>
            {(eq.by_day || []).slice().reverse().map((d) => (
              <tr key={d.date}>
                <td style={S.td}>{d.date}</td>
                <td style={{ ...S.td, color: col(d.pnl), fontWeight: 700, textAlign: 'right' }}>
                  {money(d.pnl)}
                </td>
              </tr>
            ))}
            {(!eq.by_day || eq.by_day.length === 0) && (
              <tr><td style={{ ...S.td, color: '#64748b' }}>No days with P&amp;L yet.</td></tr>
            )}
          </tbody>
        </table>
      </div>

      {/* ── Reconstruction disagreement ────────────────────── */}
      {eq.available && Math.abs(net - real) > 0.01 && (
        <div style={S.warnBox}>
          <div style={S.warnTitle}>RECONSTRUCTION DISAGREES WITH THE BROKER</div>
          <div style={S.warnBody}>
            FIFO-matching our own fill log gives <b style={{ color: col(real) }}>{money(real)}</b>,
            but Alpaca's equity curve says <b style={{ color: col(net) }}>{money(net)}</b> — a
            gap of <b>{money(net - real)}</b>
            {net >= 0 && real < 0 ? ', including a disagreement about the sign' : ''}. The
            broker is right. Our reconstruction is only as good as the quantities we recorded,
            and {dq.partial_qty_rows || 0} of those were captured mid-fill. Treat everything
            below as cost attribution, not as a result.
          </div>
        </div>
      )}

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

function Mini({ label, value, color, sub }) {
  return (
    <div>
      <div style={S.miniLabel}>{label}</div>
      <div style={{ ...S.miniValue, color: color || '#e5e7eb' }}>{value}</div>
      {sub && <div style={S.miniSub}>{sub}</div>}
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
  dcBox: { background: '#0d1320', border: '1px solid', borderRadius: 10, padding: '12px 14px', marginTop: 14 },
  dcHead: { display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 10, flexWrap: 'wrap', gap: 8 },
  dcTitle: { fontSize: 11, fontWeight: 800, color: '#94a3b8', letterSpacing: 0.6 },
  dcBadge: { fontSize: 10.5, fontWeight: 800, padding: '3px 10px', borderRadius: 6 },
  dcGrid: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(110px, 1fr))', gap: 10 },
  miniLabel: { fontSize: 9.5, color: '#64748b', textTransform: 'uppercase', letterSpacing: 0.4 },
  miniValue: { fontSize: 16, fontWeight: 800, marginTop: 3 },
  miniSub: { fontSize: 10, color: '#64748b', marginTop: 2 },
  dcNote: { fontSize: 11.5, color: '#94a3b8', marginTop: 10, lineHeight: 1.55, borderTop: '1px solid #16202f', paddingTop: 9 },
  recBox: { background: 'rgba(59,130,246,0.07)', borderLeft: '3px solid #3b82f6', borderRadius: 8, padding: '10px 13px', marginTop: 11 },
  recTitle: { fontSize: 10.5, fontWeight: 800, color: '#93c5fd', letterSpacing: 0.5 },
  recBody: { fontSize: 12, color: '#cbd5e1', marginTop: 5, lineHeight: 1.55 },
  recStats: { display: 'flex', gap: 20, flexWrap: 'wrap', marginTop: 8, fontSize: 12, color: '#cbd5e1' },
  muted: { color: '#9ca3af', padding: 40, textAlign: 'center' },
  err: { color: '#dc2626', padding: 40, textAlign: 'center' },
};

export default BrokerPanel;
