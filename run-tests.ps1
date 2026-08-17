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

# Capture cargo's exit code, NOT the pipeline's.
#
# This used to run `cargo test 2>&1 | tail -40`, so $LASTEXITCODE was tail's
# status — which is 0 whether or not the tests passed. The script therefore
# printed "All tests passed" unconditionally, including for builds that failed
# to compile. A harness that cannot report failure is worse than no harness,
# because it is trusted.
$inner = 'cargo test > /tmp/test-out 2>&1; code=$?; tail -60 /tmp/test-out; exit $code'
docker run --rm -v "${root}:/build" -w /build rust:1.95-bookworm sh -c $inner
$testExit = $LASTEXITCODE

if ($testExit -ne 0) {
    Write-Host "`nTESTS FAILED (exit $testExit)" -ForegroundColor Red
    exit $testExit
}
Write-Host "`nAll tests passed." -ForegroundColor Green
