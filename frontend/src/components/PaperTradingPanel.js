import React, { useEffect, useRef, useState } from 'react';

const WS_URL = process.env.REACT_APP_WS_URL || 'ws://localhost:8000';

/* ─── Sparkline ──────────────────────────────────────────────── */
function Sparkline({ values, width = 160, height = 32, color = '#3b82f6' }) {
  if (!values || values.length < 2) return null;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const pts = values.map((v, i) => {
    const x = (i / (values.length - 1)) * width;
    const y = height - ((v - min) / range) * (height - 4) - 2;
    return `${x},${y}`;
  }).join(' ');
  return (
    <svg width={width} height={height} style={{ display: 'block' }}>
      <polyline fill="none" stroke={color} strokeWidth="1.5" points={pts} />
    </svg>
  );
}

/* ─── Progress toward 50% target ─────────────────────────────── */
function TargetProgress({ portfolio }) {
  if (!portfolio) return null;
  const { total_pnl_pct, target_pct, progress_pct, total_value, target_value } = portfolio;
  const isUp = total_pnl_pct >= 0;
  const barColor = progress_pct >= 100 ? '#22c55e' : isUp ? '#3b82f6' : '#ef4444';
  const barWidth = Math.min(progress_pct, 100);

  return (
    <div style={{ padding: '6px 0', borderBottom: '1px solid #1e293b' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.6rem', color: '#64748b', marginBottom: 3 }}>
        <span>TARGET: +{target_pct}% (${target_value})</span>
        <span style={{ fontWeight: 700, color: progress_pct >= 100 ? '#22c55e' : '#94a3b8' }}>
          {progress_pct >= 100 ? '🎯 TARGET HIT!' : `${progress_pct.toFixed(1)}%`}
        </span>
      </div>
      <div style={{ height: 6, background: '#1e293b', borderRadius: 3, overflow: 'hidden' }}>
        <div style={{
          height: '100%', width: `${barWidth}%`, background: barColor,
          borderRadius: 3, transition: 'width 0.5s ease',
        }} />
      </div>
    </div>
  );
}

/* ─── Portfolio Header ───────────────────────────────────────── */
function PortfolioHeader({ portfolio, stats, valueHistory }) {
  if (!portfolio) return null;
  const isUp = portfolio.total_pnl >= 0;
  const clr = isUp ? '#22c55e' : '#ef4444';

  return (
    <div style={{ borderBottom: '1px solid #1e293b', paddingBottom: 8 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div>
          <div style={{ fontSize: '0.55rem', color: '#475569', textTransform: 'uppercase', letterSpacing: '0.05em' }}>Portfolio Value</div>
          <div style={{ fontSize: '1.5rem', fontWeight: 800, color: '#e2e8f0' }}>
            ${portfolio.total_value.toFixed(2)}
          </div>
          <div style={{ fontSize: '0.85rem', fontWeight: 700, color: clr }}>
            {isUp ? '+' : ''}{portfolio.total_pnl.toFixed(2)}
            <span style={{ fontSize: '0.7rem', marginLeft: 4 }}>
              ({isUp ? '+' : ''}{portfolio.total_pnl_pct.toFixed(2)}%)
            </span>
          </div>
        </div>
        <div style={{ textAlign: 'right' }}>
          <Sparkline values={valueHistory} color={clr} />
          <div style={{ fontSize: '0.55rem', color: '#475569', marginTop: 2 }}>
            {stats?.total_trades || 0} trades | {stats?.win_rate?.toFixed(0) || 0}% win
            {stats?.streak > 0 && (
              <span style={{ marginLeft: 4, color: stats.streak_type === 'W' ? '#22c55e' : '#ef4444' }}>
                {stats.streak}{stats.streak_type}
              </span>
            )}
          </div>
        </div>
      </div>
      <div style={{
        display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 6, marginTop: 6,
      }}>
        {[
          { label: 'Cash', value: `$${portfolio.cash.toFixed(2)}`, color: '#94a3b8' },
          { label: 'Invested', value: `$${portfolio.positions_value.toFixed(2)}`, color: '#3b82f6' },
          { label: 'Realized', value: `${portfolio.realized_pnl >= 0 ? '+' : ''}$${portfolio.realized_pnl.toFixed(2)}`, color: portfolio.realized_pnl >= 0 ? '#22c55e' : '#ef4444' },
        ].map((item, i) => (
          <div key={i} style={{ textAlign: 'center', padding: '4px 0', background: '#0d1320', borderRadius: 4 }}>
            <div style={{ fontSize: '0.5rem', color: '#475569', textTransform: 'uppercase' }}>{item.label}</div>
            <div style={{ fontSize: '0.75rem', fontWeight: 700, color: item.color }}>{item.value}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

/* ─── Symbol Grid ────────────────────────────────────────────── */
function SymbolGrid({ symbols }) {
  if (!symbols || symbols.length === 0) return null;
  return (
    <div style={{ borderBottom: '1px solid #1e293b', paddingBottom: 6 }}>
      <div style={{ fontSize: '0.55rem', color: '#475569', textTransform: 'uppercase', fontWeight: 600, marginBottom: 4 }}>
        Tracking 5 Stocks
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 3 }}>
        {symbols.map((s) => {
          const sc = s.direction === 'bullish' ? '#22c55e' : s.direction === 'bearish' ? '#ef4444' : '#64748b';
          const bg = s.in_position
            ? (s.position_pnl >= 0 ? 'rgba(34,197,94,0.1)' : 'rgba(239,68,68,0.1)')
            : '#0d1320';
          return (
            <div key={s.symbol} style={{
              textAlign: 'center', padding: '4px 1px', borderRadius: 4,
              background: bg, border: `1px solid ${s.in_position ? sc + '44' : '#1e293b'}`,
              position: 'relative',
            }}>
              {s.in_position && (
                <div style={{
                  position: 'absolute', top: 1, right: 2,
                  width: 4, height: 4, borderRadius: '50%', background: '#3b82f6',
                }} />
              )}
              <div style={{ fontSize: '0.6rem', fontWeight: 700, color: '#e2e8f0' }}>{s.symbol}</div>
              <div style={{ fontSize: '0.7rem', fontWeight: 700, color: '#e2e8f0' }}>
                ${s.price?.toFixed(2)}
              </div>
              <div style={{ fontSize: '0.5rem', fontWeight: 600, color: sc }}>
                {s.signal > 0 ? '▲' : s.signal < 0 ? '▼' : '·'} {(s.micro_momentum || 0).toFixed(3)}%
              </div>
              {s.in_position && (
                <div style={{
                  fontSize: '0.5rem', fontWeight: 700,
                  color: s.position_pnl >= 0 ? '#22c55e' : '#ef4444',
                }}>
                  {s.position_pnl >= 0 ? '+' : ''}${s.position_pnl.toFixed(3)}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ─── Open Positions ─────────────────────────────────────────── */
function PositionsTable({ positions }) {
  if (!positions || positions.length === 0) {
    return (
      <div style={{ padding: '8px 0', textAlign: 'center', color: '#334155', fontSize: '0.7rem' }}>
        Scanning for momentum entries...
      </div>
    );
  }
  return (
    <div style={{ borderBottom: '1px solid #1e293b', paddingBottom: 4 }}>
      <div style={{ fontSize: '0.55rem', color: '#475569', textTransform: 'uppercase', fontWeight: 600, marginBottom: 3 }}>
        Open Positions
      </div>
      {positions.map((p) => {
        const c = p.unrealized_pnl >= 0 ? '#22c55e' : '#ef4444';
        return (
          <div key={p.symbol} style={{
            display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            padding: '4px 6px', marginBottom: 2, borderRadius: 4,
            background: p.unrealized_pnl >= 0 ? 'rgba(34,197,94,0.06)' : 'rgba(239,68,68,0.06)',
          }}>
            <div>
              <span style={{ fontWeight: 700, color: '#e2e8f0', fontSize: '0.75rem' }}>{p.symbol}</span>
              <span style={{ color: '#475569', fontSize: '0.6rem', marginLeft: 4 }}>
                {p.shares}sh @ ${p.entry_price}
              </span>
            </div>
            <div style={{ textAlign: 'right' }}>
              <div style={{ fontSize: '0.75rem', fontWeight: 700, color: c }}>
                {p.unrealized_pnl >= 0 ? '+' : ''}${p.unrealized_pnl.toFixed(3)}
              </div>
              <div style={{ fontSize: '0.5rem', color: '#475569' }}>
                {p.unrealized_pnl_pct >= 0 ? '+' : ''}{p.unrealized_pnl_pct.toFixed(3)}% | {p.hold_seconds}s
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

/* ─── Trade Log ──────────────────────────────────────────────── */
function TradeLog({ trades }) {
  if (!trades || trades.length === 0) {
    return (
      <div style={{ padding: '10px 0', textAlign: 'center', color: '#334155', fontSize: '0.7rem' }}>
        No trades yet — AI analyzing patterns...
      </div>
    );
  }
  return (
    <div>
      <div style={{ fontSize: '0.55rem', color: '#475569', textTransform: 'uppercase', fontWeight: 600, marginBottom: 3 }}>
        Trade Log ({trades.length})
      </div>
      <div style={{ maxHeight: 180, overflowY: 'auto' }}>
        {trades.map((t, i) => {
          const isBuy = t.action === 'BUY';
          const ac = isBuy ? '#3b82f6' : (t.pnl >= 0 ? '#22c55e' : '#ef4444');
          const time = new Date(t.time).toLocaleTimeString();
          return (
            <div key={i} style={{
              display: 'flex', alignItems: 'center', gap: 4,
              padding: '3px 0', borderBottom: '1px solid #1e293b11', fontSize: '0.65rem',
            }}>
              <span style={{
                padding: '0px 4px', borderRadius: 2, fontSize: '0.55rem', fontWeight: 700,
                background: isBuy ? 'rgba(59,130,246,0.15)' : (t.pnl >= 0 ? 'rgba(34,197,94,0.15)' : 'rgba(239,68,68,0.15)'),
                color: ac, minWidth: 28, textAlign: 'center',
              }}>
                {t.action}
              </span>
              <span style={{ fontWeight: 700, color: '#e2e8f0', minWidth: 36 }}>{t.symbol}</span>
              <span style={{ color: '#475569', flex: 1 }}>
                {t.shares}×${t.price}
              </span>
              {t.pnl != null && (
                <span style={{ fontWeight: 700, color: t.pnl >= 0 ? '#22c55e' : '#ef4444', minWidth: 50, textAlign: 'right' }}>
                  {t.pnl >= 0 ? '+' : ''}${(t.pnl ?? 0).toFixed(3)}
                </span>
              )}
              {t.pnl == null && (
                <span style={{ color: '#334155', minWidth: 50, textAlign: 'right' }}>${(t.total ?? 0).toFixed(2)}</span>
              )}
              <span style={{ color: '#1e293b', fontSize: '0.5rem', minWidth: 48, textAlign: 'right' }}>{time}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ─── Stats Footer ───────────────────────────────────────────── */
function StatsFooter({ portfolio, stats }) {
  if (!portfolio || !stats) return null;
  return (
    <div style={{
      display: 'flex', justifyContent: 'space-between', alignItems: 'center',
      padding: '4px 0', borderTop: '1px solid #1e293b', marginTop: 4,
      fontSize: '0.55rem', color: '#334155',
    }}>
      <span>DD: <span style={{ color: '#ef4444' }}>{portfolio.drawdown_pct.toFixed(2)}%</span></span>
      <span>Avg hold: {stats.avg_hold_seconds}s</span>
      <span>Pos: {stats.open_positions}/5</span>
      <span style={{
        padding: '1px 5px', borderRadius: 3, fontWeight: 700,
        background: 'rgba(234,179,8,0.1)', color: '#eab308', fontSize: '0.5rem',
      }}>$100 CHALLENGE</span>
    </div>
  );
}

/* ═════════════════════════════════════════════════════════════════
   MAIN — PAPER TRADING PANEL
   ═════════════════════════════════════════════════════════════════ */
function PaperTradingPanel() {
  const [data, setData] = useState(null);
  const [connected, setConnected] = useState(false);
  const [valueHistory, setValueHistory] = useState([]);
  const wsRef = useRef(null);

  useEffect(() => {
    let disposed = false;
    let reconnectTimer;

    const connect = () => {
      const ws = new WebSocket(`${WS_URL}/ws/paper-trade`);
      wsRef.current = ws;
      ws.onopen = () => { if (!disposed) setConnected(true); };
      ws.onmessage = (evt) => {
        if (disposed) return;
        const d = JSON.parse(evt.data);
        setData(d);
        // Use server-side value_history if available
        if (d.value_history && d.value_history.length > 0) {
          setValueHistory(d.value_history);
        }
      };
      ws.onclose = () => {
        if (!disposed) { setConnected(false); reconnectTimer = setTimeout(connect, 1000); }
      };
      ws.onerror = () => ws.close();
    };
    connect();

    return () => {
      disposed = true;
      if (wsRef.current) wsRef.current.close();
      if (reconnectTimer) clearTimeout(reconnectTimer);
    };
  }, []);

  return (
    <div style={{
      background: '#0f1724', borderRadius: 8, border: '1px solid #1e293b',
      padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 6,
      height: '100%', overflow: 'hidden',
    }}>
      {/* Title */}
      <div style={{
        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
        paddingBottom: 4, borderBottom: '1px solid #1e293b',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
          <span style={{
            width: 6, height: 6, borderRadius: '50%',
            background: connected ? '#22c55e' : '#ef4444',
            animation: connected ? 'pulse 2s infinite' : 'none',
          }} />
          <span style={{ fontSize: '0.8rem', fontWeight: 800, color: '#eab308', letterSpacing: '0.05em' }}>
            $100 → $150 CHALLENGE
          </span>
        </div>
        <span style={{
          padding: '1px 6px', borderRadius: 4, fontSize: '0.55rem', fontWeight: 700,
          background: 'rgba(168,85,247,0.12)', color: '#a855f7',
        }}>
          AI SCALPER
        </span>
      </div>

      {!data ? (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div style={{ textAlign: 'center', color: '#475569' }}>
            <div style={{ fontSize: '1.5rem', marginBottom: 4 }}>⚡</div>
            <div style={{ fontSize: '0.75rem' }}>Starting $100 challenge...</div>
            <div style={{ fontSize: '0.6rem', color: '#334155', marginTop: 2 }}>
              Loading NVDA · AAPL · MSFT · GOOGL · AMZN
            </div>
          </div>
        </div>
      ) : (
        <>
          <TargetProgress portfolio={data.portfolio} />
          <PortfolioHeader portfolio={data.portfolio} stats={data.stats} valueHistory={valueHistory} />
          <SymbolGrid symbols={data.symbols} />
          <PositionsTable positions={data.positions} />
          <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
            <TradeLog trades={data.recent_trades} />
          </div>
          <StatsFooter portfolio={data.portfolio} stats={data.stats} />
        </>
      )}
    </div>
  );
}

export default PaperTradingPanel;
