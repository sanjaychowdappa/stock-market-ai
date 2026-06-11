"""
Launcher — runs both Kronos server (port 8001) and Agent server (port 8002)
in the same container, sharing the GPU.
"""
import subprocess
import sys
import os
import signal

procs = []

def cleanup(signum, frame):
    for p in procs:
        p.terminate()
    sys.exit(0)

signal.signal(signal.SIGTERM, cleanup)
signal.signal(signal.SIGINT, cleanup)

# Start Kronos server on port 8001
p1 = subprocess.Popen([
    sys.executable, "-m", "uvicorn",
    "kronos_server:app",
    "--host", "0.0.0.0", "--port", "8001",
    "--log-level", "info",
])
procs.append(p1)

# Start Agent server on port 8002
p2 = subprocess.Popen([
    sys.executable, "-m", "uvicorn",
    "agent_server:app",
    "--host", "0.0.0.0", "--port", "8002",
    "--log-level", "info",
])
procs.append(p2)

print("Both servers started: Kronos=8001, Agents=8002")

# Wait for either to exit
try:
    for p in procs:
        p.wait()
except KeyboardInterrupt:
    cleanup(None, None)
