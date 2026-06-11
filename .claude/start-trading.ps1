# Auto-start trading system on session start
# Launches Docker Desktop, waits for engine, starts all services

# Check if Docker is already running
$dockerRunning = $false
try {
    docker info 2>$null | Select-Object -First 1
    if ($?) { $dockerRunning = $true }
} catch {}

if (-not $dockerRunning) {
    # Launch Docker Desktop
    Start-Process "C:\Program Files\Docker\Docker\Docker Desktop.exe" -ErrorAction SilentlyContinue

    # Wait up to 2 minutes for Docker engine
    for ($i = 0; $i -lt 24; $i++) {
        Start-Sleep -Seconds 5
        try {
            docker info 2>$null | Select-Object -First 1
            if ($?) { $dockerRunning = $true; break }
        } catch {}
    }
}

if ($dockerRunning) {
    Set-Location "C:\Users\sanja\stock-market-ai"
    docker compose up -d 2>$null
    Write-Output '{"systemMessage":"Trading system started: backend + frontend + redis + sidecar on localhost:3000 and localhost:8000"}'
} else {
    Write-Output '{"systemMessage":"Docker Desktop failed to start within 2 minutes. Run docker compose up -d manually."}'
}
