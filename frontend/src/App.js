import React, { useState, useEffect, useCallback } from 'react';
import axios from 'axios';
import CandlestickChart from './components/CandlestickChart';
import SignalPanel from './components/SignalPanel';
import PredictionPanel from './components/PredictionPanel';
import PatternPanel from './components/PatternPanel';
import StopLossReport from './components/StopLossReport';
import Watchlist from './components/Watchlist';
import AgentDashboard from './components/AgentDashboard';
import LiveTicker from './components/LiveTicker';
import './App.css';

const API = process.env.REACT_APP_API_URL || 'http://localhost:8000/api';

function App() {
  const [symbol, setSymbol] = useState('AAPL');
  const [inputSymbol, setInputSymbol] = useState('AAPL');
  const [period, setPeriod] = useState('1d');
  const [interval, setChartInterval] = useState('5m');
  const [stockData, setStockData] = useState(null);
  const [signals, setSignals] = useState(null);
  const [prediction, setPrediction] = useState(null);
  const [patterns, setPatterns] = useState(null);
  const [stopLoss, setStopLoss] = useState(null);
  const [fullAnalysis, setFullAnalysis] = useState(null);
  const [livePrice, setLivePrice] = useState(null);
  const [loading, setLoading] = useState(false);
  const [activeTab, setActiveTab] = useState('chart');
  const [error, setError] = useState(null);

  const fetchLivePrice = useCallback(async (sym) => {
    try {
      const res = await axios.get(`${API}/stocks/${sym}/live`);
      setLivePrice(res.data);
    } catch (e) { /* silent */ }
  }, []);

  const fetchAllData = useCallback(async (sym) => {
    setLoading(true);
    setError(null);
    try {
      const [stockRes, signalRes, predRes, patternRes, stopLossRes, analysisRes] = await Promise.allSettled([
        axios.get(`${API}/stocks/${sym}?period=${period}&interval=${interval}`),
        axios.get(`${API}/signals/${sym}`),
        axios.get(`${API}/predictions/${sym}`),
        axios.get(`${API}/patterns/${sym}`),
        axios.get(`${API}/signals/${sym}/stop-loss`),
        axios.get(`${API}/signals/${sym}/full-analysis`),
      ]);

      if (stockRes.status === 'fulfilled') setStockData(stockRes.value.data);
      if (signalRes.status === 'fulfilled') setSignals(signalRes.value.data);
      if (predRes.status === 'fulfilled') setPrediction(predRes.value.data);
      if (patternRes.status === 'fulfilled') setPatterns(patternRes.value.data);
      if (stopLossRes.status === 'fulfilled') setStopLoss(stopLossRes.value.data);
      if (analysisRes.status === 'fulfilled') setFullAnalysis(analysisRes.value.data);
    } catch (err) {
      setError('Failed to fetch data. Make sure backend is running.');
    }
    setLoading(false);
  }, [period, interval]);

  useEffect(() => {
    fetchAllData(symbol);
    fetchLivePrice(symbol);
    const dataTimer = setInterval(() => fetchAllData(symbol), 30000);
    const liveTimer = setInterval(() => fetchLivePrice(symbol), 15000);
    return () => { clearInterval(dataTimer); clearInterval(liveTimer); };
  }, [symbol, fetchAllData, fetchLivePrice]);

  const handleSearch = (e) => {
    e.preventDefault();
    if (inputSymbol.trim()) {
      setSymbol(inputSymbol.trim().toUpperCase());
    }
  };

  const INTERVAL_OPTIONS = {
    '1d':  [{ v: '1m', l: '1 Min' }, { v: '5m', l: '5 Min' }, { v: '15m', l: '15 Min' }],
    '5d':  [{ v: '5m', l: '5 Min' }, { v: '15m', l: '15 Min' }, { v: '1h', l: '1 Hour' }],
    '1mo': [{ v: '15m', l: '15 Min' }, { v: '1h', l: '1 Hour' }, { v: '1d', l: 'Daily' }],
    '3mo': [{ v: '1h', l: '1 Hour' }, { v: '1d', l: 'Daily' }],
    '6mo': [{ v: '1d', l: 'Daily' }, { v: '1wk', l: 'Weekly' }],
    '1y':  [{ v: '1d', l: 'Daily' }, { v: '1wk', l: 'Weekly' }],
    '2y':  [{ v: '1d', l: 'Daily' }, { v: '1wk', l: 'Weekly' }],
  };

  const handlePeriodChange = (newPeriod) => {
    setPeriod(newPeriod);
    const opts = INTERVAL_OPTIONS[newPeriod];
    if (opts && !opts.find(o => o.v === interval)) {
      setChartInterval(opts[0].v);
    }
  };

  const agentRec = fullAnalysis?.agent_recommendation;

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
        <LiveTicker data={livePrice} />
        <div className="header-right">
          <select value={period} onChange={(e) => handlePeriodChange(e.target.value)} className="select">
            <option value="1d">Today</option>
            <option value="5d">5D</option>
            <option value="1mo">1M</option>
            <option value="3mo">3M</option>
            <option value="6mo">6M</option>
            <option value="1y">1Y</option>
            <option value="2y">2Y</option>
          </select>
          <select value={interval} onChange={(e) => setChartInterval(e.target.value)} className="select">
            {(INTERVAL_OPTIONS[period] || []).map(o => (
              <option key={o.v} value={o.v}>{o.l}</option>
            ))}
          </select>
        </div>
      </header>

      {error && <div className="error-banner">{error}</div>}

      {agentRec && (
        <div className={`agent-banner ${agentRec.action.toLowerCase().replace(' ', '-')}`}>
          <div className="agent-banner-content">
            <span className="agent-badge">AI Agent</span>
            <span className="agent-action">{agentRec.action}</span>
            <span className="agent-summary">{agentRec.summary}</span>
            <span className="agent-score">Score: {agentRec.score}</span>
          </div>
        </div>
      )}

      <nav className="tabs">
        {[
          { id: 'chart', label: 'Chart & Signals' },
          { id: 'patterns', label: 'Patterns' },
          { id: 'prediction', label: 'Prediction' },
          { id: 'stoploss', label: 'Stop Loss' },
          { id: 'agent', label: 'Agent Dashboard' },
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
        {loading && <div className="loading-overlay"><div className="spinner" /></div>}

        {activeTab === 'chart' && (
          <div className="chart-layout">
            <div className="chart-area">
              <CandlestickChart
                data={stockData?.data}
                patterns={patterns?.patterns}
                prediction={prediction}
                symbol={symbol}
                interval={interval}
              />
            </div>
            <div className="side-panels">
              <SignalPanel signals={signals} />
              <Watchlist onSelect={(sym) => { setSymbol(sym); setInputSymbol(sym); }} />
            </div>
          </div>
        )}

        {activeTab === 'patterns' && (
          <PatternPanel patterns={patterns} symbol={symbol} />
        )}

        {activeTab === 'prediction' && (
          <PredictionPanel prediction={prediction} symbol={symbol} />
        )}

        {activeTab === 'stoploss' && (
          <StopLossReport report={stopLoss} symbol={symbol} />
        )}

        {activeTab === 'agent' && (
          <AgentDashboard analysis={fullAnalysis} symbol={symbol} />
        )}
      </main>
    </div>
  );
}

export default App;
