import React, { useState } from 'react';
import CandlestickChart from './components/CandlestickChart';
import LiveTicker from './components/LiveTicker';
import LiveTrading from './components/LiveTrading';
import MomentumPanel from './components/MomentumPanel';
import ExperimentsPanel from './components/ExperimentsPanel';
import './App.css';

function App() {
  const [symbol, setSymbol] = useState('NVDA');
  const [inputSymbol, setInputSymbol] = useState('NVDA');
  const [activeTab, setActiveTab] = useState('momentum');

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

      <nav className="tabs">
        {[
          { id: 'momentum', label: 'Momentum Portfolio' },
          { id: 'experiments', label: 'Experiments (A/B)' },
          { id: 'live-trading', label: 'Signal Trader (legacy)' },
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
        {activeTab === 'momentum' && (
          <MomentumPanel />
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
