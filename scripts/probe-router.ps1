#!/usr/bin/env pwsh
# T-025 — Empirical router probe
# Tests the b9196-cpu build in router mode against prepared fixtures.
# Answers the five questions T-023 could only infer from --help.

$ErrorActionPreference = "Stop"
$LLAMA = "$env:LOCALAPPDATA\LlamaManager\runtimes\b9196-cpu\llama-server.exe"
$FIXTURES = "E:\LMM\fixtures"
$PORT = 8080
$SERVER = "http://127.0.0.1:$PORT"
$RESULTS = "probe-results.json"
$script:probe_num = 0
$script:all_results = @()

function Write-Result($question, $answer, $detail) {
    $obj = @{ question = $question; answer = $answer; detail = $detail }
    $script:all_results += $obj
    Write-Output "  [$question] $answer"
    if ($detail) { Write-Output "    $detail" }
}

function Wait-Server($timeout = 15) {
    $elapsed = 0
    while ($elapsed -lt $timeout) {
        try {
            $r = Invoke-WebRequest -Uri "$SERVER/health" -TimeoutSec 2 -UseBasicParsing
            if ($r.StatusCode -eq 200) { return $true }
        } catch {
            Start-Sleep -Seconds 1
            $elapsed++
        }
    }
    return $false
}

function Stop-Server($proc) {
    if ($proc) {
        if ($proc.HasExited) {
            $proc.Dispose()
        } else {
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
            try { $proc.WaitForExit(5000) } catch {}
            $proc.Dispose()
        }
    }
    Start-Sleep -Seconds 1
}

function Start-Router($arg_str) {
    $script:probe_num = $script:probe_num + 1
    Write-Output "Starting: $LLAMA $arg_str"
    # Use cmd to parse arguments correctly
    $proc = Start-Process -FilePath "cmd.exe" `
        -ArgumentList "/c `"$LLAMA`" $arg_str" `
        -NoNewWindow -PassThru `
        -RedirectStandardOutput "server-$script:probe_num.log" `
        -RedirectStandardError "server-$script:probe_num-err.log"
    return $proc
}

Write-Output "=== T-025 Empirical Router Probe ==="
Write-Output "Build: b9196-cpu"
Write-Output "Time: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Write-Output ""

# Clean up any existing server
$existing = Get-Process -Name llama-server -ErrorAction SilentlyContinue
Stop-Server $existing

# ============ Q1: --models-preset with absolute path ============
Write-Output "Q1: Does --models-preset register a model by absolute path?"

# Create preset with absolute path - in a clean directory
if (Test-Path "C:\temp\probe-q1") { Remove-Item "C:\temp\probe-q1" -Recurse -Force }
New-Item -ItemType Directory -Path "C:\temp\probe-q1" -Force | Out-Null
$ini = "[test-external]`nmodel = E:\LMM\fixtures\model-a.gguf`n"
$ini | Out-File -FilePath "C:\temp\probe-q1\preset.ini" -Encoding ascii

$proc = Start-Router("--models-preset C:\temp\probe-q1\preset.ini --port $PORT")
if (Wait-Server) {
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/v1/models" -UseBasicParsing
        $body = $r.Content
        Write-Output "  Models response: $body"
        if ($body -match "test-external") {
            Write-Result "Q1a" "yes" "Absolute path in preset registered model 'test-external'"
        } else {
            Write-Result "Q1a" "no" "Absolute path in preset did NOT register. Response: $body"
        }
    } catch {
        Write-Result "Q1a" "error" $_
    }
} else {
    Write-Result "Q1a" "no-start" ""
}
Stop-Server $proc

# Test non-existent path
Write-Output "  Testing non-existent path..."
$ini2 = "[test-missing]`nmodel = C:\nonexistent\model.gguf`n"
$ini2 | Out-File -FilePath "C:\temp\probe-q1\preset-missing.ini" -Encoding ascii
$proc = Start-Router("--models-preset C:\temp\probe-q1\preset-missing.ini --port $PORT")
if (Wait-Server) {
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/v1/models" -UseBasicParsing
        if ($r.Content -match "test-missing") {
            Write-Result "Q1b" "yes" "Non-existent path registered (lazy loading)"
        } else {
            Write-Result "Q1b" "no" "Non-existent path NOT registered"
        }
    } catch {
        Write-Result "Q1b" "error" $_
    }
} else {
    Write-Result "Q1b" "no-start" ""
}
Stop-Server $proc

# ============ Q2: --models-dir scan follows reparse points ============
Write-Output ""
Write-Output "Q2: Does --models-dir scan follow reparse points?"

# Setup: real file, symlink, junction
if (Test-Path "C:\temp\probe-q2") { Remove-Item "C:\temp\probe-q2" -Recurse -Force }
New-Item -ItemType Directory -Path "C:\temp\probe-q2" -Force | Out-Null
New-Item -ItemType Directory -Path "C:\temp\probe-q2-real" -Force | Out-Null
New-Item -ItemType Directory -Path "C:\temp\probe-q2-jtarget" -Force | Out-Null

Copy-Item "$FIXTURES\model-a.gguf" "C:\temp\probe-q2-real\real.gguf" -Force
Copy-Item "$FIXTURES\model-b.gguf" "C:\temp\probe-q2-jtarget\junction-model.gguf" -Force

# Create symlink (file)
try {
    New-Item -ItemType SymbolicLink -Path "C:\temp\probe-q2\symlink.gguf" `
        -Target "C:\temp\probe-q2-real\real.gguf" -Force | Out-Null
    Write-Output "  Symlink created"
} catch {
    Write-Output "  Symlink failed: $_"
}

# Create junction (directory)
try {
    cmd /c "mklink /J C:\temp\probe-q2\jdir C:\temp\probe-q2-jtarget" | Out-Null
    Write-Output "  Junction created"
} catch {
    Write-Output "  Junction failed: $_"
}

$proc = Start-Router("--models-dir C:\temp\probe-q2 --port $PORT")
if (Wait-Server) {
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/v1/models" -UseBasicParsing
        $body = $r.Content
        Write-Output "  Models response: $body"
        
        if ($body -match "symlink") {
            Write-Result "Q2a" "yes" "File symlink followed and registered"
        } else {
            Write-Result "Q2a" "no" "File symlink NOT followed"
        }
        
        if ($body -match "junction-model") {
            Write-Result "Q2b" "yes" "Directory junction followed and registered"
        } else {
            Write-Result "Q2b" "no" "Directory junction NOT followed"
        }
    } catch {
        Write-Result "Q2" "error" $_
    }
} else {
    Write-Result "Q2" "no-start" ""
}
Stop-Server $proc

# ============ Q3: Scan depth ============
Write-Output ""
Write-Output "Q3: What is the scan's real depth?"

if (Test-Path "C:\temp\probe-q3") { Remove-Item "C:\temp\probe-q3" -Recurse -Force }
New-Item -ItemType Directory -Path "C:\temp\probe-q3\sub1\sub2" -Force | Out-Null
Copy-Item "$FIXTURES\model-a.gguf" "C:\temp\probe-q3\top.gguf" -Force
Copy-Item "$FIXTURES\model-b.gguf" "C:\temp\probe-q3\sub1\level1.gguf" -Force
Copy-Item "$FIXTURES\model-c.gguf" "C:\temp\probe-q3\sub1\sub2\level2.gguf" -Force

$proc = Start-Router("--models-dir C:\temp\probe-q3 --port $PORT")
if (Wait-Server) {
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/v1/models" -UseBasicParsing
        $body = $r.Content
        Write-Output "  Models response: $body"
        
        if ($body -match "top") {
            Write-Result "Q3a" "yes" "Top-level model registered"
        } else {
            Write-Result "Q3a" "no" "Top-level model NOT registered"
        }
        
        if ($body -match "level1") {
            Write-Result "Q3b" "yes" "One level down registered"
        } else {
            Write-Result "Q3b" "no" "One level down NOT registered"
        }
        
        if ($body -match "level2") {
            Write-Result "Q3c" "yes" "Two levels down registered"
        } else {
            Write-Result "Q3c" "no" "Two levels down NOT registered"
        }
    } catch {
        Write-Result "Q3" "error" $_
    }
} else {
    Write-Result "Q3" "no-start" ""
}
Stop-Server $proc

# ============ Q4: Projector on preset channel ============
Write-Output ""
Write-Output "Q4: How is a projector expressed on the PresetDeclaresPath channel?"

# Create multi-file model directory
if (Test-Path "C:\temp\probe-q4") { Remove-Item "C:\temp\probe-q4" -Recurse -Force }
New-Item -ItemType Directory -Path "C:\temp\probe-q4" -Force | Out-Null
Copy-Item "$FIXTURES\model-a.gguf" "C:\temp\probe-q4\vision-model.gguf" -Force
Copy-Item "$FIXTURES\model-b.gguf" "C:\temp\probe-q4\mmproj-vision.gguf" -Force

# Try preset with mmproj key
$ini = "[vision-test]`nmodel = C:\temp\probe-q4\vision-model.gguf`nmmproj = C:\temp\probe-q4\mmproj-vision.gguf`n"
$ini | Out-File -FilePath "C:\temp\probe-q4\preset-mmproj.ini" -Encoding ascii

$proc = Start-Router("--models-preset C:\temp\probe-q4\preset-mmproj.ini --port $PORT")
if (Wait-Server) {
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/v1/models" -UseBasicParsing
        $body = $r.Content
        Write-Output "  Models response: $body"
        
        if ($body -match "vision-test") {
            Write-Result "Q4" "yes" "Multi-file model with mmproj registered via preset. Syntax: [name] model=path mmproj=path"
        } else {
            Write-Result "Q4" "no" "Multi-file model NOT registered. Response: $body"
        }
    } catch {
        Write-Result "Q4" "error" $_
    }
} else {
    Write-Result "Q4" "no-start" ""
}
Stop-Server $proc

# ============ Q5: Health endpoint with no model loaded ============
Write-Output ""
Write-Output "Q5: Which health endpoint is reliable with no model loaded?"

$proc = Start-Router("--port $PORT")
if (Wait-Server) {
    Write-Output "  Server started with no models loaded"
    
    # Test /health
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/health" -UseBasicParsing
        Write-Result "Q5a" "yes" "/health responded: $($r.StatusCode) - $($r.Content)"
    } catch {
        Write-Result "Q5a" "no" "/health failed: $_"
    }
    
    # Test /props
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/props" -UseBasicParsing
        Write-Result "Q5b" "yes" "/props responded: $($r.StatusCode)"
    } catch {
        Write-Result "Q5b" "no" "/props failed: $_"
    }
    
    # Test /v1/models
    try {
        $r = Invoke-WebRequest -Uri "$SERVER/v1/models" -UseBasicParsing
        Write-Result "Q5c" "yes" "/v1/models responded: $($r.StatusCode) - $($r.Content)"
    } catch {
        Write-Result "Q5c" "no" "/v1/models failed: $_"
    }
} else {
    Write-Result "Q5" "no-start" ""
}
Stop-Server $proc

# ============ Save results ============
Write-Output ""
Write-Output "=== Probe complete ==="
$script:all_results | ConvertTo-Json -Depth 5 | Out-File -FilePath $RESULTS -Encoding utf8
Write-Output "Results written to: $RESULTS"
