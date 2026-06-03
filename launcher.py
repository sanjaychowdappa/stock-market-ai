"""
Stock Market AI — Desktop Launcher
===================================
One-click start/stop for the entire trading system.
Shows live status, backend logs, and EOD reports.
"""

import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox
import subprocess
import threading
import os
import sys
import json
import time
from datetime import datetime
from pathlib import Path

# Project root = directory containing this script
PROJECT_DIR = Path(__file__).resolve().parent
REPORTS_DIR = PROJECT_DIR / "reports"
COMPOSE_FILE = PROJECT_DIR / "docker-compose.yml"


class StockMarketLauncher(tk.Tk):
    def __init__(self):
        super().__init__()

        self.title("Stock Market AI — Trading System")
        self.geometry("900x700")
        self.minsize(750, 550)
        self.configure(bg="#1a1a2e")

        # State
        self.running = False
        self.log_process = None
        self.status_thread = None
        self._stop_polling = threading.Event()

        # Style
        self.style = ttk.Style()
        self.style.theme_use("clam")
        self._configure_styles()

        # Build UI
        self._build_header()
        self._build_status_bar()
        self._build_controls()
        self._build_notebook()
        self._build_footer()

        # Check initial state
        self.after(500, self._check_initial_state)

        # Handle window close
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    def _configure_styles(self):
        bg = "#1a1a2e"
        fg = "#e0e0e0"
        accent = "#0f3460"
        green = "#00c853"
        red = "#ff1744"

        self.style.configure("Dark.TFrame", background=bg)
        self.style.configure("Dark.TLabel", background=bg, foreground=fg, font=("Segoe UI", 10))
        self.style.configure("Title.TLabel", background=bg, foreground="#00d4ff",
                             font=("Segoe UI", 18, "bold"))
        self.style.configure("Subtitle.TLabel", background=bg, foreground="#8888aa",
                             font=("Segoe UI", 9))
        self.style.configure("Status.TLabel", background=bg, foreground=fg,
                             font=("Segoe UI", 10, "bold"))
        self.style.configure("Green.TLabel", background=bg, foreground=green,
                             font=("Segoe UI", 11, "bold"))
        self.style.configure("Red.TLabel", background=bg, foreground=red,
                             font=("Segoe UI", 11, "bold"))
        self.style.configure("Start.TButton", font=("Segoe UI", 12, "bold"), padding=12)
        self.style.configure("Stop.TButton", font=("Segoe UI", 12, "bold"), padding=12)
        self.style.configure("Action.TButton", font=("Segoe UI", 10), padding=6)

        self.style.configure("Dark.TNotebook", background=bg)
        self.style.configure("Dark.TNotebook.Tab", background=accent, foreground=fg,
                             font=("Segoe UI", 10), padding=[12, 4])
        self.style.map("Dark.TNotebook.Tab",
                        background=[("selected", "#16213e")],
                        foreground=[("selected", "#00d4ff")])

    def _build_header(self):
        header = ttk.Frame(self, style="Dark.TFrame")
        header.pack(fill=tk.X, padx=20, pady=(15, 5))

        ttk.Label(header, text="Stock Market AI", style="Title.TLabel").pack(side=tk.LEFT)
        ttk.Label(header, text="Rust Engine + Kronos ONNX  |  $100 -> $150 Challenge",
                  style="Subtitle.TLabel").pack(side=tk.LEFT, padx=(15, 0), pady=(5, 0))

    def _build_status_bar(self):
        bar = ttk.Frame(self, style="Dark.TFrame")
        bar.pack(fill=tk.X, padx=20, pady=5)

        self.status_dot = tk.Canvas(bar, width=14, height=14, bg="#1a1a2e", highlightthickness=0)
        self.status_dot.pack(side=tk.LEFT)
        self._draw_dot("#666666")

        self.status_label = ttk.Label(bar, text="Checking...", style="Status.TLabel")
        self.status_label.pack(side=tk.LEFT, padx=(8, 20))

        self.backend_label = ttk.Label(bar, text="Backend: —", style="Dark.TLabel")
        self.backend_label.pack(side=tk.LEFT, padx=(0, 15))

        self.frontend_label = ttk.Label(bar, text="Frontend: —", style="Dark.TLabel")
        self.frontend_label.pack(side=tk.LEFT, padx=(0, 15))

        self.redis_label = ttk.Label(bar, text="Redis: —", style="Dark.TLabel")
        self.redis_label.pack(side=tk.LEFT)

    def _build_controls(self):
        ctrl = ttk.Frame(self, style="Dark.TFrame")
        ctrl.pack(fill=tk.X, padx=20, pady=8)

        self.start_btn = tk.Button(ctrl, text="  START TRADING  ", font=("Segoe UI", 13, "bold"),
                                    bg="#00c853", fg="white", activebackground="#00e676",
                                    relief=tk.FLAT, cursor="hand2", command=self._start_system)
        self.start_btn.pack(side=tk.LEFT, padx=(0, 10))

        self.stop_btn = tk.Button(ctrl, text="  STOP  ", font=("Segoe UI", 13, "bold"),
                                   bg="#ff1744", fg="white", activebackground="#ff5252",
                                   relief=tk.FLAT, cursor="hand2", command=self._stop_system,
                                   state=tk.DISABLED)
        self.stop_btn.pack(side=tk.LEFT, padx=(0, 10))

        tk.Button(ctrl, text=" Open Dashboard ", font=("Segoe UI", 10),
                  bg="#0f3460", fg="white", activebackground="#16213e",
                  relief=tk.FLAT, cursor="hand2",
                  command=lambda: os.startfile("http://localhost:3000")
                  ).pack(side=tk.LEFT, padx=(20, 10))

        tk.Button(ctrl, text=" EOD Report ", font=("Segoe UI", 10),
                  bg="#0f3460", fg="white", activebackground="#16213e",
                  relief=tk.FLAT, cursor="hand2",
                  command=self._generate_eod_report
                  ).pack(side=tk.LEFT, padx=(0, 10))

        tk.Button(ctrl, text=" Open Reports ", font=("Segoe UI", 10),
                  bg="#0f3460", fg="white", activebackground="#16213e",
                  relief=tk.FLAT, cursor="hand2",
                  command=self._open_reports_dir
                  ).pack(side=tk.LEFT)

    def _build_notebook(self):
        self.notebook = ttk.Notebook(self, style="Dark.TNotebook")
        self.notebook.pack(fill=tk.BOTH, expand=True, padx=20, pady=(5, 10))

        # Logs tab
        log_frame = tk.Frame(self.notebook, bg="#0d0d1a")
        self.notebook.add(log_frame, text="  Live Logs  ")

        self.log_text = scrolledtext.ScrolledText(log_frame, bg="#0d0d1a", fg="#00ff88",
                                                   font=("Consolas", 9), insertbackground="#00ff88",
                                                   wrap=tk.WORD, state=tk.DISABLED,
                                                   relief=tk.FLAT, borderwidth=8)
        self.log_text.pack(fill=tk.BOTH, expand=True)
        self.log_text.tag_configure("info", foreground="#00ff88")
        self.log_text.tag_configure("warn", foreground="#ffd740")
        self.log_text.tag_configure("error", foreground="#ff5252")
        self.log_text.tag_configure("trade_buy", foreground="#00e5ff")
        self.log_text.tag_configure("trade_sell", foreground="#ff80ab")
        self.log_text.tag_configure("system", foreground="#b388ff")

        # EOD Report tab
        report_frame = tk.Frame(self.notebook, bg="#0d0d1a")
        self.notebook.add(report_frame, text="  EOD Report  ")

        self.report_text = scrolledtext.ScrolledText(report_frame, bg="#0d0d1a", fg="#e0e0e0",
                                                      font=("Consolas", 9), insertbackground="#e0e0e0",
                                                      wrap=tk.WORD, state=tk.DISABLED,
                                                      relief=tk.FLAT, borderwidth=8)
        self.report_text.pack(fill=tk.BOTH, expand=True)

        # Config tab
        config_frame = tk.Frame(self.notebook, bg="#0d0d1a")
        self.notebook.add(config_frame, text="  Config  ")

        config_text = tk.Text(config_frame, bg="#0d0d1a", fg="#e0e0e0",
                              font=("Consolas", 10), relief=tk.FLAT, borderwidth=8)
        config_text.pack(fill=tk.BOTH, expand=True)
        config_text.insert(tk.END, f"""Stock Market AI — Configuration
================================

Project Directory:  {PROJECT_DIR}
Compose File:      {COMPOSE_FILE}
Reports Directory:  {REPORTS_DIR}

Services:
  Backend (Rust):     Port 8000
  Frontend (React):   Port 3000
  Redis:              Port 6379
  Finetune Sidecar:   Port 8001 (GPU, EOD only)

Symbols:  NVDA, AAPL, MSFT, GOOGL, AMZN
Challenge: $100 -> $150 paper trading

Market Hours (ET):
  Pre-market:    04:00 - 09:30
  Regular:       09:30 - 16:00
  After-hours:   16:00 - 20:00

EOD Report: Auto-publishes at 4:05 PM ET
            Also available via the "EOD Report" button
            Saved to: {REPORTS_DIR}

Endpoints:
  Health:     http://localhost:8000/api/health
  Signals:    http://localhost:8000/api/signals/NVDA
  Patterns:   http://localhost:8000/api/patterns/NVDA
  Daily:      http://localhost:8000/api/daily/status
  EOD Report: http://localhost:8000/api/eod/report
  Dashboard:  http://localhost:3000
""")
        config_text.configure(state=tk.DISABLED)

    def _build_footer(self):
        footer = ttk.Frame(self, style="Dark.TFrame")
        footer.pack(fill=tk.X, padx=20, pady=(0, 10))

        self.time_label = ttk.Label(footer, text="", style="Subtitle.TLabel")
        self.time_label.pack(side=tk.RIGHT)
        self._update_clock()

    def _update_clock(self):
        now = datetime.now()
        self.time_label.config(text=now.strftime("%Y-%m-%d  %H:%M:%S"))
        self.after(1000, self._update_clock)

    def _draw_dot(self, color):
        self.status_dot.delete("all")
        self.status_dot.create_oval(2, 2, 12, 12, fill=color, outline=color)

    # ─── Docker Operations ─────────────────────────────────────

    def _run_docker(self, args, capture=True):
        """Run a docker compose command."""
        cmd = ["docker", "compose", "-f", str(COMPOSE_FILE)] + args
        try:
            if capture:
                result = subprocess.run(cmd, capture_output=True, text=True, timeout=300,
                                         cwd=str(PROJECT_DIR))
                return result
            else:
                return subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                         text=True, cwd=str(PROJECT_DIR))
        except FileNotFoundError:
            self._log("Docker not found! Please install Docker Desktop.", "error")
            return None
        except subprocess.TimeoutExpired:
            self._log("Docker command timed out.", "error")
            return None

    def _check_initial_state(self):
        """Check if containers are already running."""
        def check():
            result = self._run_docker(["ps", "--format", "json"])
            if result and result.returncode == 0 and result.stdout.strip():
                # Parse running containers
                lines = result.stdout.strip().split('\n')
                running_services = []
                for line in lines:
                    try:
                        data = json.loads(line)
                        if data.get("State") == "running":
                            running_services.append(data.get("Service", ""))
                    except (json.JSONDecodeError, KeyError):
                        pass

                if "backend" in running_services:
                    self.after(0, lambda: self._set_running_state(True))
                    self.after(0, lambda: self._log("System already running — attached.", "system"))
                    self.after(0, self._start_log_stream)
                    self.after(0, lambda: self._start_status_polling())
                    return

            self.after(0, lambda: self._set_running_state(False))

        threading.Thread(target=check, daemon=True).start()

    def _start_system(self):
        """Start all Docker services."""
        self.start_btn.config(state=tk.DISABLED, text="  STARTING...  ", bg="#888888")
        self._log("Starting Stock Market AI trading system...", "system")

        def start():
            # Start core services first
            self._log("Building & starting backend, frontend, redis...", "system")
            result = self._run_docker(["up", "-d", "--build", "backend", "frontend", "redis"])

            if result and result.returncode == 0:
                self.after(0, lambda: self._log("Core services started!", "info"))
                self.after(0, lambda: self._set_running_state(True))
                self.after(0, self._start_log_stream)
                self.after(0, lambda: self._start_status_polling())

                # Build finetune-sidecar in background (non-critical)
                self._log("Building finetune-sidecar (background, non-blocking)...", "system")
                sidecar = self._run_docker(["up", "-d", "--build", "finetune-sidecar"])
                if sidecar and sidecar.returncode == 0:
                    self.after(0, lambda: self._log("Finetune sidecar started.", "info"))
                else:
                    self.after(0, lambda: self._log("Finetune sidecar failed (non-critical).", "warn"))
            else:
                err = result.stderr if result else "Docker not available"
                self.after(0, lambda e=err: self._log(f"Failed to start: {e}", "error"))
                self.after(0, lambda: self._set_running_state(False))

        threading.Thread(target=start, daemon=True).start()

    def _stop_system(self):
        """Stop all Docker services."""
        self.stop_btn.config(state=tk.DISABLED, text="  STOPPING...  ", bg="#888888")
        self._log("Stopping trading system...", "system")
        self._stop_polling.set()

        def stop():
            # Kill log stream
            if self.log_process:
                try:
                    self.log_process.terminate()
                except Exception:
                    pass
                self.log_process = None

            result = self._run_docker(["down"])
            if result and result.returncode == 0:
                self.after(0, lambda: self._log("System stopped.", "system"))
            else:
                self.after(0, lambda: self._log("Stop may have had issues.", "warn"))

            self.after(0, lambda: self._set_running_state(False))

        threading.Thread(target=stop, daemon=True).start()

    def _set_running_state(self, running):
        """Update UI to reflect running state."""
        self.running = running
        if running:
            self._draw_dot("#00c853")
            self.status_label.config(text="RUNNING")
            self.start_btn.config(state=tk.DISABLED, text="  RUNNING  ", bg="#555555")
            self.stop_btn.config(state=tk.NORMAL, text="  STOP  ", bg="#ff1744")
        else:
            self._draw_dot("#ff1744")
            self.status_label.config(text="STOPPED")
            self.start_btn.config(state=tk.NORMAL, text="  START TRADING  ", bg="#00c853")
            self.stop_btn.config(state=tk.DISABLED, text="  STOP  ", bg="#555555")
            self.backend_label.config(text="Backend: --")
            self.frontend_label.config(text="Frontend: --")
            self.redis_label.config(text="Redis: --")

    # ─── Log Streaming ─────────────────────────────────────────

    def _start_log_stream(self):
        """Stream Docker logs into the log panel."""
        if self.log_process:
            try:
                self.log_process.terminate()
            except Exception:
                pass

        def stream():
            proc = self._run_docker(["logs", "-f", "--tail", "100", "backend"], capture=False)
            if not proc:
                return
            self.log_process = proc

            try:
                for line in iter(proc.stdout.readline, ''):
                    if not self.running:
                        break
                    cleaned = line.strip()
                    if cleaned:
                        # Determine tag based on content
                        tag = "info"
                        if "WARN" in cleaned:
                            tag = "warn"
                        elif "ERROR" in cleaned:
                            tag = "error"
                        elif "PAPER: BUY" in cleaned:
                            tag = "trade_buy"
                        elif "PAPER: SELL" in cleaned:
                            tag = "trade_sell"
                        elif "EOD" in cleaned or "Fine-tune" in cleaned:
                            tag = "system"

                        self.after(0, lambda l=cleaned, t=tag: self._append_log(l, t))
            except Exception:
                pass

        threading.Thread(target=stream, daemon=True).start()

    def _append_log(self, text, tag="info"):
        self.log_text.configure(state=tk.NORMAL)
        self.log_text.insert(tk.END, text + "\n", tag)
        self.log_text.see(tk.END)
        self.log_text.configure(state=tk.DISABLED)

    def _log(self, text, tag="info"):
        ts = datetime.now().strftime("%H:%M:%S")
        self._append_log(f"[{ts}] {text}", tag)

    # ─── Status Polling ────────────────────────────────────────

    def _start_status_polling(self):
        self._stop_polling.clear()

        def poll():
            while not self._stop_polling.is_set():
                try:
                    result = self._run_docker(["ps", "--format", "json"])
                    if result and result.returncode == 0:
                        lines = result.stdout.strip().split('\n')
                        services = {}
                        for line in lines:
                            try:
                                data = json.loads(line)
                                svc = data.get("Service", "")
                                state = data.get("State", "")
                                services[svc] = state
                            except (json.JSONDecodeError, KeyError):
                                pass

                        be = services.get("backend", "down")
                        fe = services.get("frontend", "down")
                        rd = services.get("redis", "down")

                        self.after(0, lambda: self.backend_label.config(
                            text=f"Backend: {be}",
                            foreground="#00c853" if be == "running" else "#ff1744"))
                        self.after(0, lambda: self.frontend_label.config(
                            text=f"Frontend: {fe}",
                            foreground="#00c853" if fe == "running" else "#ff1744"))
                        self.after(0, lambda: self.redis_label.config(
                            text=f"Redis: {rd}",
                            foreground="#00c853" if rd == "running" else "#ff1744"))
                except Exception:
                    pass

                self._stop_polling.wait(10)

        threading.Thread(target=poll, daemon=True).start()

    # ─── EOD Report ────────────────────────────────────────────

    def _generate_eod_report(self):
        """Fetch EOD report from backend and display it."""
        if not self.running:
            messagebox.showwarning("Not Running", "Start the trading system first.")
            return

        self._log("Fetching EOD report...", "system")

        def fetch():
            try:
                import urllib.request
                req = urllib.request.Request("http://localhost:8000/api/eod/report")
                with urllib.request.urlopen(req, timeout=15) as resp:
                    data = json.loads(resp.read().decode())

                text_summary = data.get("text_summary", "No report data.")
                saved_to = data.get("saved_to", "unknown")

                self.after(0, lambda: self._show_report(text_summary))
                self.after(0, lambda: self._log(f"EOD report generated. Saved to: {saved_to}", "system"))

            except Exception as e:
                self.after(0, lambda: self._log(f"Failed to fetch EOD report: {e}", "error"))

        threading.Thread(target=fetch, daemon=True).start()

    def _show_report(self, text):
        """Display report in the report tab."""
        self.report_text.configure(state=tk.NORMAL)
        self.report_text.delete("1.0", tk.END)
        self.report_text.insert(tk.END, text)
        self.report_text.configure(state=tk.DISABLED)
        self.notebook.select(1)  # Switch to report tab

    def _open_reports_dir(self):
        """Open the reports directory in file explorer."""
        REPORTS_DIR.mkdir(exist_ok=True)
        os.startfile(str(REPORTS_DIR))

    # ─── Cleanup ───────────────────────────────────────────────

    def _on_close(self):
        """Clean shutdown."""
        self._stop_polling.set()
        if self.log_process:
            try:
                self.log_process.terminate()
            except Exception:
                pass
        self.destroy()


if __name__ == "__main__":
    app = StockMarketLauncher()
    app.mainloop()
