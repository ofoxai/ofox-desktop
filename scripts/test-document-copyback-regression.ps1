# Native Windows negative control for the fixed-fixture copyback cancellation
# tests. This intentionally changes four guards, restores them in finally, and
# never installs agents, opens accounts, or calls a model. Run explicitly on a
# clean checkout after the full Windows suite, before packaging that checkout.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutDirectory)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "Native Windows is required; cross-compilation is not regression evidence."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$patchRelative = "scripts/fixtures/document-copyback-cancel-guards.patch"
$sourceRelative = "src-tauri/src/services/assistant/documents/windows_sandbox.rs"
$testPrefix = "services::assistant::documents::windows_sandbox::tests::"
$cases = @(
    @{ name = "after-exit"; test = "cancelling_after_exit_stops_all_copyback_publication"; marker = "COPYBACK_CANCEL_REGRESSION_AFTER_EXIT" },
    @{ name = "between-files"; test = "cancelling_between_copyback_files_stops_remaining_publication"; marker = "COPYBACK_CANCEL_REGRESSION_BETWEEN_FILES" },
    @{ name = "before-rename"; test = "cancelling_after_staging_fsync_does_not_publish_the_staged_file"; marker = "COPYBACK_CANCEL_REGRESSION_BEFORE_RENAME" }
)
$steps = New-Object 'System.Collections.Generic.List[object]'
$originalLocation = Get-Location
$originalEncoding = [Console]::OutputEncoding
$startedAt = [DateTime]::UtcNow
$reportDirectory = [IO.Path]::GetFullPath($OutDirectory)
$patchApplied = $false
$restored = $false
$positiveProbePassed = $false
$redCount = 0
$greenCount = 0
$success = $false
$failure = $null
$commit = $null
$patchSha256 = $null
$outputCreated = $false

function Resolve-NativeCommand([string]$Name) {
    $command = Get-Command $Name -CommandType Application -ErrorAction Stop | Select-Object -First 1
    return $command.Source
}

function Invoke-Logged([string]$Name, [string]$Program, [string[]]$Arguments) {
    $logPath = Join-Path $reportDirectory "$Name.log"
    $exitCode = -1
    $lines = @()
    $previousPreference = $ErrorActionPreference
    $timer = [Diagnostics.Stopwatch]::StartNew()
    Write-Host "[$Name]"
    try {
        # PowerShell 5.1 wraps native stderr in ErrorRecords. Ordinary Cargo
        # progress must not be mistaken for a failing regression assertion.
        $ErrorActionPreference = "Continue"
        $lines = @(& $Program @Arguments 2>&1 | ForEach-Object { $_.ToString() } | Tee-Object -FilePath $logPath)
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
        $timer.Stop()
        $steps.Add([pscustomobject]@{
            name = $Name
            exitCode = $exitCode
            durationSeconds = [Math]::Round($timer.Elapsed.TotalSeconds, 2)
            log = [IO.Path]::GetFileName($logPath)
        })
    }
    return [pscustomobject]@{ exitCode = $exitCode; output = ($lines -join "`n") }
}

function Read-Git([string[]]$Arguments) {
    $output = @(& $git @Arguments)
    if ($LASTEXITCODE -ne 0) { throw "Git verification failed." }
    return ($output -join "`n").Trim()
}

function Assert-TrackedClean {
    $status = Read-Git @("status", "--porcelain=v1", "--untracked-files=no")
    if ($status) { throw "Tracked checkout is dirty; do not package or overwrite it." }
    if ($commit -and (Read-Git @("rev-parse", "HEAD")) -ne $commit) {
        throw "Checkout identity changed during regression verification."
    }
}

function Assert-ExactPass([object]$Result, [string]$Name) {
    if ($Result.exitCode -ne 0 -or $Result.output -notmatch '(?m)^running 1 test\s*$' -or
        $Result.output -notmatch 'test result: ok\. 1 passed; 0 failed; 0 ignored;') {
        throw "$Name did not execute exactly one passing native test. See its log."
    }
}

function Test-Arguments([string]$Name) {
    return @("test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "$testPrefix$Name", "--", "--exact", "--nocapture", "--test-threads=1")
}

try {
    Set-Location $repoRoot
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    $git = Resolve-NativeCommand "git"
    $cargo = Resolve-NativeCommand "cargo"
    $rustc = Resolve-NativeCommand "rustc"
    $node = Resolve-NativeCommand "node"
    Assert-TrackedClean
    $commit = Read-Git @("rev-parse", "HEAD")
    if ($commit -notmatch '^[a-f0-9]{40}$') { throw "Invalid checkout identity." }
    $null = Read-Git @("ls-files", "--error-unmatch", "--", $patchRelative, $sourceRelative)
    # Keep logs out of the compiled checkout identity. This accepts a new path
    # outside the checkout or an ignored path such as artifacts/windows-tests/.
    if ($reportDirectory.StartsWith($repoRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        $relativeOutput = $reportDirectory.Substring($repoRoot.Length + 1)
        & $git check-ignore --quiet -- $relativeOutput
        if ($LASTEXITCODE -ne 0) { throw "Use an output directory outside the checkout or a Git-ignored path." }
    }
    if (Test-Path -LiteralPath $reportDirectory) { throw "Use a new output directory to preserve earlier evidence." }
    New-Item -ItemType Directory -Path $reportDirectory | Out-Null
    $outputCreated = $true
    $patch = Join-Path $repoRoot $patchRelative
    $source = Join-Path $repoRoot $sourceRelative
    $patchSha256 = (Get-FileHash -LiteralPath $patch -Algorithm SHA256).Hash.ToLowerInvariant()
    $sourceSha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash

    $toolchain = Invoke-Logged "rust-toolchain" $rustc @("-vV")
    if ($toolchain.exitCode -ne 0 -or $toolchain.output -notmatch '(?m)^host: \S+-pc-windows-msvc\s*$') {
        throw "Use the native Windows MSVC Rust toolchain."
    }
    $prepare = Invoke-Logged "assistant-prepare" $node @("scripts/assistant-prepare.mjs")
    if ($prepare.exitCode -ne 0) { throw "Pinned public runtime preparation failed." }
    Assert-TrackedClean
    $probe = Invoke-Logged "positive-full-probe" $cargo (Test-Arguments "document_lpac_proves_python_office_and_escape_denials")
    Assert-ExactPass $probe "Complete LPAC/Python/OfficeCLI probe"
    $positiveProbePassed = $true

    try {
        $check = Invoke-Logged "reverse-check" $git @("apply", "--reverse", "--check", $patchRelative)
        if ($check.exitCode -ne 0) { throw "The reviewed negative-control patch no longer matches the source." }
        $reverse = Invoke-Logged "reverse-guards" $git @("apply", "--reverse", $patchRelative)
        if ($reverse.exitCode -ne 0) { throw "Could not apply the negative control." }
        $patchApplied = $true
        foreach ($case in $cases) {
            $red = Invoke-Logged ("red-" + $case.name) $cargo (Test-Arguments $case.test)
            if ($red.exitCode -ne 101 -or $red.output -notmatch '(?m)^running 1 test\s*$' -or
                $red.output -notmatch 'test result: FAILED\. 0 passed; 1 failed; 0 ignored;' -or
                $red.output -notmatch ([regex]::Escape($case.marker))) {
                throw "Negative control $($case.name) did not fail its explicit cancellation assertion; probe, compile and unrelated failures do not count as RED."
            }
            $redCount++
        }
    } finally {
        if ($patchApplied) {
            $check = Invoke-Logged "restore-check" $git @("apply", "--check", $patchRelative)
            if ($check.exitCode -ne 0) { throw "SOURCE_RESTORE_FAILED: patch no longer applies; preserve the checkout for review." }
            $restore = Invoke-Logged "restore-guards" $git @("apply", $patchRelative)
            if ($restore.exitCode -ne 0) { throw "SOURCE_RESTORE_FAILED: do not package this checkout." }
            $patchApplied = $false
        }
        Assert-TrackedClean
        if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $sourceSha256) {
            throw "SOURCE_RESTORE_FAILED: source bytes differ from the original clean checkout."
        }
        $restored = $true
    }

    foreach ($case in $cases) {
        $green = Invoke-Logged ("green-" + $case.name) $cargo (Test-Arguments $case.test)
        Assert-ExactPass $green ("Restored " + $case.name)
        $greenCount++
    }
    Assert-TrackedClean
    $success = $positiveProbePassed -and $restored -and $redCount -eq 3 -and $greenCount -eq 3
} catch {
    $failure = $_.Exception.Message
    Write-Host "FAILED: $failure" -ForegroundColor Red
} finally {
    if ($outputCreated) {
        $summary = [ordered]@{
            schemaVersion = 1
            scope = "native-copyback-cancel-red-green"
            nativeWindows = $true
            commit = $commit
            patchSha256 = $patchSha256
            positiveFullProbePassed = $positiveProbePassed
            expectedRedCount = $redCount
            restoredTrackedClean = $restored
            greenCount = $greenCount
            paidModelCalls = 0
            success = $success
            failure = $failure
            startedAtUtc = $startedAt.ToString("o")
            finishedAtUtc = [DateTime]::UtcNow.ToString("o")
            steps = @($steps.ToArray())
        }
        [IO.File]::WriteAllText((Join-Path $reportDirectory "summary.json"), ($summary | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    }
    [Console]::OutputEncoding = $originalEncoding
    Set-Location $originalLocation
}
if (-not $success) { throw "Native copyback cancellation regression verification failed: $failure" }
Write-Host "Native copyback cancellation RED/GREEN passed; source restored and tracked clean." -ForegroundColor Green
