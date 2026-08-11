import React, { useState, useEffect } from 'react';
import CandlestickChart from './components/CandlestickChart';
import LiveTicker from './components/LiveTicker';
import LiveTrading from './components/LiveTrading';
import MomentumPanel from './components/MomentumPanel';
import AgenticPanel from './components/AgenticPanel';
import ExperimentsPanel from './components/ExperimentsPanel';
import BrokerPanel from './components/BrokerPanel';
import HaltBanner from './components/HaltBanner';
import './App.css';

function App() {
  // The charted symbol must come from whatever the backend is actually
  // streaming, never a hard-coded name.
  //
  // This was pinned to 'NVDA'. Once the sector-leader agent started choosing
  // the universe, NVDA stopped being tracked — no engine, no stream — so the
  // chart sat on "Connecting…" forever and looked broken. The data was fine;
  // the UI was asking for a symbol that no longer existed.
  const [symbol, setSymbol] = useState(null);
  const [inputSymbol, setInputSymbol] = useState('');
  const [universe, setUniverse] = useState([]);
  // Real broker P&L is the landing tab: the first number seen should be the
  // one that survives contact with a real exchange, not a simulator's estimate.
  const [activeTab, setActiveTab] = useState('broker');

  useEffect(() => {
    const API = process.env.REACT_APP_API_URL || 'http://127.0.0.1:8000/api';
    fetch(`${API}/health`)
      .then((r) => r.json())
      .then((h) => {
        const syms = h.symbols || [];
        setUniverse(syms);
        // Only adopt a default once, and only if the user has not chosen one.
        setSymbol((cur) => cur ?? syms[0] ?? null);
        setInputSymbol((cur) => cur || syms[0] || '');
      })
      .catch(() => {});
  }, []);

  const handleSearch = (e) => {
    e.preventDefault();
    if (inputSymbol.trim()) {
      setSymbol(inputSymbol.trim().toUpperCase());
    }
  };

  return (
    <div className="app">
      <header className="header">
        <div className="header-left">
          <h1 className="logo">StockAI<span>Agent</span></h1>
        </div>
        <form className="search-form" onSubmit={handleSearch}>
          {/* Datalist of what is actually streaming. Typing a symbol the
              backend does not track leaves the chart on "Connecting…"
              forever, which is exactly how this bug presented. */}
          <input
            type="text"
            list="tradeable-universe"
            value={inputSymbol}
            onChange={(e) => setInputSymbol(e.target.value.toUpperCase())}
            placeholder="Symbol…"
            className="search-input"
          />
          <datalist id="tradeable-universe">
            {universe.map((s) => <option key={s} value={s} />)}
          </datalist>
          <button type="submit" className="search-btn">Analyze</button>
          {universe.length > 0 && !universe.includes(symbol) && (
            <span style={{ fontSize: 11, color: '#f59e0b', marginLeft: 8 }}>
              {symbol} is not in the traded universe — no live data
            </span>
          )}
        </form>
        {symbol && <LiveTicker symbol={symbol} />}
      </header>

      {/* Above the tabs on purpose: during a halt every position panel fills
          with simulator holdings while the broker is flat, and that must not
          require opening a tab to understand. */}
      <HaltBanner />

      <nav className="tabs">
        {[
          { id: 'broker', label: 'Real P&L (Alpaca)' },
          { id: 'momentum', label: 'Momentum Portfolio' },
          { id: 'agentic', label: 'Agentic Module' },
          { id: 'experiments', label: 'Experiments (A/B)' },
          { id: 'live-trading', label: 'Signal Trader (decisions only)' },
          { id: 'chart', label: 'Chart' },
        ].map((tab) => (
          <button
            key={tab.id}
            className={`tab ${activeTab === tab.id ? 'active' : ''}`}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </nav>

      <main className="main-content">
        {activeTab === 'broker' && (
          <BrokerPanel />
        )}

        {activeTab === 'momentum' && (
          <MomentumPanel />
        )}

        {activeTab === 'agentic' && (
          <AgenticPanel />
        )}

        {activeTab === 'experiments' && (
          <ExperimentsPanel />
        )}

        {activeTab === 'live-trading' && (
          symbol ? <LiveTrading symbol={symbol} /> : <div style={{padding:20,color:"#9ca3af"}}>Loading universe…</div>
        )}

        {activeTab === 'chart' && (
          <div className="chart-layout">
            <div className="chart-area">
              {symbol ? <CandlestickChart symbol={symbol} /> : <div style={{padding:20,color:"#9ca3af"}}>Loading universe…</div>}
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
