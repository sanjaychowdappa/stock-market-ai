import React, { useState } from 'react';
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
  const [symbol, setSymbol] = useState('NVDA');
  const [inputSymbol, setInputSymbol] = useState('NVDA');
  // Real broker P&L is the landing tab: the first number seen should be the
  // one that survives contact with a real exchange, not a simulator's estimate.
  const [activeTab, setActiveTab] = useState('broker');

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
          <input
            type="text"
            value={inputSymbol}
            onChange={(e) => setInputSymbol(e.target.value.toUpperCase())}
            placeholder="Enter symbol..."
            className="search-input"
          />
          <button type="submit" className="search-btn">Analyze</button>
        </form>
        <LiveTicker symbol={symbol} />
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
          <LiveTrading symbol={symbol} />
        )}

        {activeTab === 'chart' && (
          <div className="chart-layout">
            <div className="chart-area">
              <CandlestickChart symbol={symbol} />
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
