# Run in Windows PowerShell 5.1 or PowerShell 7. Does not install/uninstall tools.
[CmdletBinding()]
param(
    [switch]$BackendOnly,
    [switch]$SkipDependencyInstall,
    [switch]$BuildApp,
    [string]$OutputDirectory = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "Run this script on a Windows host with the MSVC Rust toolchain."
}
if ($BackendOnly -and $BuildApp) {
    throw "-BuildApp requires frontend checks; omit -BackendOnly."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repoRoot ("artifacts/windows-tests/" + (Get-Date -Format "yyyyMMdd-HHmmss") + "-$PID")
}
$reportDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $reportDirectory | Out-Null
$steps = New-Object 'System.Collections.Generic.List[object]'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("ofox-windows-tests-" + [guid]::NewGuid().ToString("N"))
$originalLocation = Get-Location
$startedAt = [DateTime]::UtcNow
$originalConsoleEncoding = [Console]::OutputEncoding
$verificationSucceeded = $false
$failure = $null

function Resolve-NativeCommand([string]$Name) {
    # Prefer pnpm.cmd over pnpm.ps1 so execution policies do not block it.
    $candidates = if ($Name -eq "pnpm") { @("pnpm.cmd", "pnpm") } else { @($Name) }
    foreach ($candidate in $candidates) {
        $command = Get-Command $candidate -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($command) { return $command.Source }
    }
    throw "Missing command '$Name'. See docs/windows-testing.md for prerequisites."
}

function Invoke-VerificationStep([string]$Name, [string]$Program, [string[]]$Arguments) {
    if (-not (Test-Path -LiteralPath $Program -PathType Leaf)) {
        throw "Executable not found: $Program"
    }
    $logPath = Join-Path $reportDirectory "$Name.log"
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $exitCode = -1
    Write-Host "`n[$Name] $Program $($Arguments -join ' ')"
    $previousErrorPreference = $ErrorActionPreference
    try {
        # Windows PowerShell 5.1 treats native stderr as ErrorRecords. Cargo
        # sends ordinary progress there, so decide success by its exit code.
        $ErrorActionPreference = "Continue"
        & $Program @Arguments 2>&1 | ForEach-Object { $_.ToString() } | Tee-Object -FilePath $logPath
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorPreference
        $timer.Stop()
        $steps.Add([pscustomobject]@{
            name = $Name
            exitCode = $exitCode
            durationSeconds = [Math]::Round($timer.Elapsed.TotalSeconds, 2)
            log = [IO.Path]::GetFileName($logPath)
        })
    }
    if ($exitCode -ne 0) { throw "$Name failed (exit $exitCode). See $logPath" }
}

try {
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    Set-Location $repoRoot
    $cargo = Resolve-NativeCommand "cargo"
    $rustc = Resolve-NativeCommand "rustc"
    Invoke-VerificationStep "rust-toolchain" $rustc @("-vV")
    $rustHost = Get-Content (Join-Path $reportDirectory "rust-toolchain.log") -Raw
    if ($rustHost -notmatch "host: \S+-pc-windows-msvc") {
        throw "Use a native Windows MSVC Rust toolchain, not a GNU or cross-compilation toolchain."
    }
    $powerShell = (Get-Process -Id $PID).Path
    Invoke-VerificationStep "runner-self-tests" $powerShell @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $repoRoot "scripts/tests/test-windows-runner.ps1"))

    if (-not $BackendOnly) {
        $node = Resolve-NativeCommand "node"
        $pnpm = Resolve-NativeCommand "pnpm"
        Invoke-VerificationStep "node-version" $node @("--version")
        Invoke-VerificationStep "pnpm-version" $pnpm @("--version")
        if (-not $SkipDependencyInstall) {
            Invoke-VerificationStep "dependencies" $pnpm @("install", "--frozen-lockfile")
        }
        Invoke-VerificationStep "typecheck" $pnpm @("typecheck")
        Invoke-VerificationStep "format" $pnpm @("format:check")
        Invoke-VerificationStep "frontend-tests" $pnpm @("test:unit")
        Invoke-VerificationStep "renderer-build" $pnpm @("build:renderer")
    } elseif (-not (Test-Path (Join-Path $repoRoot "dist"))) {
        # Tauri's backend build checks this directory even without the UI.
        New-Item -ItemType Directory -Path (Join-Path $repoRoot "dist") | Out-Null
    }

    Invoke-VerificationStep "rust-format" $cargo @("fmt", "--check", "--manifest-path", "src-tauri/Cargo.toml")
    Invoke-VerificationStep "windows-clippy" $cargo @("clippy", "--manifest-path", "src-tauri/Cargo.toml", "--all-targets", "--", "-D", "warnings")

    # Bound lifecycle tests use temporary configuration and never need real
    # accounts, keys, installed agents, Store access, or a WSL distribution.
    # Keep USERPROFILE intact so rustup still finds the host's toolchain.
    $isolatedEnvironment = @{
        CC_SWITCH_TEST_HOME = $testRoot
        APPDATA = (Join-Path $testRoot "AppData/Roaming")
        LOCALAPPDATA = (Join-Path $testRoot "AppData/Local")
        HERMES_HOME = (Join-Path $testRoot "Hermes")
    }
    $savedEnvironment = @{}
    try {
        foreach ($name in $isolatedEnvironment.Keys) {
            $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
            New-Item -ItemType Directory -Force -Path $isolatedEnvironment[$name] | Out-Null
            [Environment]::SetEnvironmentVariable($name, $isolatedEnvironment[$name], "Process")
        }
        # Environment names are case-insensitive on Windows. Save/restore this
        # once; treating NO_PROXY and no_proxy separately loses the old value.
        $savedEnvironment["NO_PROXY"] = [Environment]::GetEnvironmentVariable("NO_PROXY", "Process")
        $existingBypass = $savedEnvironment["NO_PROXY"]
        $bypass = if ($existingBypass) { "$existingBypass,127.0.0.1,localhost,::1" } else { "127.0.0.1,localhost,::1" }
        [Environment]::SetEnvironmentVariable("NO_PROXY", $bypass, "Process")
        Invoke-VerificationStep "windows-rust-tests" $cargo @("test", "--manifest-path", "src-tauri/Cargo.toml", "--", "--test-threads=1")
    } finally {
        foreach ($name in $savedEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
        }
    }

    $probeArgs = @("build", "--manifest-path", "src-tauri/Cargo.toml", "--bin", "verify_tool_lifecycle")
    if ($BuildApp) {
        Invoke-VerificationStep "desktop-build" $pnpm @("tauri", "build", "--no-bundle")
        $probeArgs += "--release"
    }
    Invoke-VerificationStep "installation-probe-build" $cargo $probeArgs
    # --help checks the entry point without probing this host's real tools.
    $profile = if ($BuildApp) { "release" } else { "debug" }
    $probe = Join-Path $repoRoot "src-tauri/target/$profile/verify_tool_lifecycle.exe"
    Invoke-VerificationStep "installation-probe-help" $probe @("--help")
    $verificationSucceeded = $true
} catch {
    $failure = $_.Exception.Message
    Write-Host "`nFAILED: $failure" -ForegroundColor Red
} finally {
    $summary = [ordered]@{
        schemaVersion = 1
        platform = "windows"
        scope = $(if ($BackendOnly) { "backend" } else { "frontend-and-backend" })
        startedAtUtc = $startedAt.ToString("o")
        finishedAtUtc = [DateTime]::UtcNow.ToString("o")
        success = $verificationSucceeded
        failure = $failure
        steps = @($steps.ToArray())
    }
    try {
        [IO.File]::WriteAllText(
            (Join-Path $reportDirectory "summary.json"),
            ($summary | ConvertTo-Json -Depth 5),
            [Text.UTF8Encoding]::new($false)
        )
    } finally {
        [Console]::OutputEncoding = $originalConsoleEncoding
        Set-Location $originalLocation
        if (Test-Path $testRoot) { Remove-Item -Recurse -Force $testRoot }
        Write-Host "`nReport: $reportDirectory"
    }
}
if (-not $verificationSucceeded) { exit 1 }
Write-Host "Windows checks passed. Native UI acceptance: docs/windows-testing.md" -ForegroundColor Green
