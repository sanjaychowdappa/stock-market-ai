import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';

// ── Suppress benign ResizeObserver loop warnings ──────────────────
// React dev overlay (react-error-overlay) treats this as a real error.
// We intercept at every level to prevent the red screen.
// This is a known browser limitation, not an app bug.
// See: https://github.com/WICG/resize-observer/issues/38

const isResizeObserverErr = (msg) =>
  typeof msg === 'string' && msg.includes('ResizeObserver');

// 1. Capture phase listener — fires before React's overlay handler
window.addEventListener('error', (e) => {
  if (isResizeObserverErr(e.message)) {
    e.stopImmediatePropagation();
    e.preventDefault();
    return;
  }
}, true);  // <-- capture phase

// 2. Bubble phase fallback
window.addEventListener('error', (e) => {
  if (isResizeObserverErr(e.message)) {
    e.stopImmediatePropagation();
    e.preventDefault();
  }
});

// 3. Classic onerror fallback
const _origErr = window.onerror;
window.onerror = (msg, ...args) => {
  if (isResizeObserverErr(msg)) return true;
  return _origErr ? _origErr(msg, ...args) : false;
};

// 4. Unhandled rejection (some browsers surface it this way)
window.addEventListener('unhandledrejection', (e) => {
  if (isResizeObserverErr(e.reason?.message || String(e.reason))) {
    e.preventDefault();
  }
});

const root = ReactDOM.createRoot(document.getElementById('root'));
root.render(<App />);
