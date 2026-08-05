# Run the Rust regression tests.
#
# Local `cargo test` does NOT work on this machine: Rust is installed but the
# MSVC linker is not, so every build script fails with "linker `link.exe` not
# found". Everything compiles in Docker instead, same as the app itself.
#
#   .\run-tests.ps1
#
# The first run pulls rust:1.95-bookworm and compiles dependencies (~2 min).
# Later runs reuse the mounted target/ directory and take seconds.

$ErrorActionPreference = "Stop"
$root = Join-Path $PSScriptRoot "backend-rust"

Write-Host "Running regression tests in Docker..." -ForegroundColor Cyan
docker run --rm -v "${root}:/build" -w /build rust:1.95-bookworm `
    sh -c "cargo test 2>&1 | tail -40"

if ($LASTEXITCODE -ne 0) {
    Write-Host "`nTESTS FAILED" -ForegroundColor Red
    exit 1
}
Write-Host "`nAll tests passed." -ForegroundColor Green
