Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

Write-Host "==> Backend: fmt/build/test/clippy"
Push-Location (Join-Path $repoRoot "backend")
cargo fmt --check
cargo build --verbose
cargo test --verbose
cargo clippy -- -D warnings
Pop-Location

Write-Host "==> Frontend: npm ci/build"
Push-Location (Join-Path $repoRoot "frontend")
if (Test-Path "package-lock.json") {
    npm ci
} else {
    npm install
}
npm run build
Pop-Location

Write-Host "Local CI parity checks completed successfully."
