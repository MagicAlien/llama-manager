# T-023 — export verified flags for the pinned build as docs/verified-flags.md.
#
# This script is the source of truth for the committed markdown: it runs the
# app's headless export subcommand, which reads the runtimes row from the
# database and renders the document. The output is a pure function of the
# stored row, so it is byte-for-byte stable — the snapshot assert below
# verifies that.
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts/export-verified-flags.ps1
#
# Requirements: llama-manager.exe must be built (cargo build in src-tauri),
# and the pinned build (b9196, cpu) must be installed and verified in the app
# database (via the installer or `llama-manager install-verify`).

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$exe = Join-Path $root "src-tauri\target\debug\llama-manager.exe"
$md = Join-Path $root "docs\verified-flags.md"
$tag = "b9196"
$backend = "cpu"

if (-not (Test-Path $exe)) {
    Write-Error "llama-manager.exe not found at $exe - run cargo build in src-tauri first"
}

# Export once.
Write-Host "Exporting verified flags for $tag/$backend..."
& $exe export-verified-flags $tag $backend $md
if ($LASTEXITCODE -ne 0) {
    Write-Error "export-verified-flags failed (exit $LASTEXITCODE)"
}

# Snapshot assert: export again to a temp file and verify byte-for-byte
# stability. The renderer is deterministic, so two exports from the same
# database row must produce identical output.
$temp = [System.IO.Path]::GetTempFileName()
try {
    & $exe export-verified-flags $tag $backend $temp
    if ($LASTEXITCODE -ne 0) {
        throw "second export failed"
    }

    $a = [System.IO.File]::ReadAllBytes($md)
    $b = [System.IO.File]::ReadAllBytes($temp)
    if ($a.Length -ne $b.Length) {
        throw "byte-stability check failed: lengths differ"
    }
    for ($i = 0; $i -lt $a.Length; $i++) {
        if ($a[$i] -ne $b[$i]) {
            throw "byte-stability check failed at byte $i"
        }
    }
    Write-Host ("Byte-stability check passed (" + $a.Length + " bytes)")
} finally {
    Remove-Item $temp -ErrorAction SilentlyContinue
}

Write-Host ("Exported " + $md)
