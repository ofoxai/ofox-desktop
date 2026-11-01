#Requires -Version 7.0
# Fail early with startup diagnostics before compiling the complete Windows backend.
$ErrorActionPreference = "Stop"
$fixtureRepo = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$fixtureNode = Join-Path $fixtureRepo "assistant-runtime/resources/node/node.exe"
$fixtureSource = Join-Path $fixtureRepo "src-tauri/tests/fixtures/assistant-process.mjs"

function Invoke-FixtureProbe([string]$Role, [bool]$PreserveSystemRoot) {
    $fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ("ofox-process-" + [guid]::NewGuid() + " 空 格")
    [IO.Directory]::CreateDirectory($fixtureRoot) | Out-Null
    $fixtureScript = Join-Path $fixtureRoot "process fixture.mjs"
    $fixtureReady = Join-Path $fixtureRoot "ready.json"
    $fixtureNonce = [guid]::NewGuid().ToString()
    Copy-Item -LiteralPath $fixtureSource -Destination $fixtureScript
    $fixtureProcess = [Diagnostics.Process]::new()
    $fixtureStarted = $false
    $fixtureDescendant = $null
    try {
        $fixtureInfo = $fixtureProcess.StartInfo
        $fixtureInfo.FileName = $fixtureNode
        $fixtureInfo.WorkingDirectory = $fixtureRoot
        $fixtureInfo.UseShellExecute = $false
        $fixtureInfo.CreateNoWindow = $true
        $fixtureInfo.RedirectStandardInput = $true
        $fixtureInfo.RedirectStandardOutput = $true
        $fixtureInfo.RedirectStandardError = $true
        $fixtureInfo.Environment.Clear()
        if ($PreserveSystemRoot) {
            $fixtureInfo.Environment["SystemRoot"] = [Environment]::GetEnvironmentVariable("SystemRoot")
        }
        foreach ($fixtureArg in @($fixtureScript, $Role, $fixtureReady, $fixtureNonce)) {
            $fixtureInfo.ArgumentList.Add($fixtureArg)
        }
        $fixtureStarted = $fixtureProcess.Start()
        if (-not $fixtureStarted) { throw "Could not start fixture Node" }
        $fixtureOutput = $fixtureProcess.StandardOutput.ReadToEndAsync()
        $fixtureError = $fixtureProcess.StandardError.ReadToEndAsync()
        $fixtureDeadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not (Test-Path -LiteralPath $fixtureReady)) {
            if ($fixtureProcess.HasExited -or [DateTime]::UtcNow -ge $fixtureDeadline) { break }
            Start-Sleep -Milliseconds 10
        }
        $fixtureOk = Test-Path -LiteralPath $fixtureReady
        if ($fixtureOk) {
            $fixtureRecord = Get-Content -LiteralPath $fixtureReady -Raw | ConvertFrom-Json
            if ($fixtureRecord.nonce -ne $fixtureNonce -or $fixtureRecord.role -ne $Role -or $fixtureProcess.HasExited) {
                throw "Fixture readiness did not identify the live process"
            }
            if ($Role -eq "parent") {
                if ($fixtureRecord.parent -ne $fixtureProcess.Id -or $fixtureRecord.child -eq $fixtureProcess.Id) {
                    throw "Fixture readiness did not identify the parent and child"
                }
                $fixtureDescendant = [Diagnostics.Process]::GetProcessById([int]$fixtureRecord.child)
                if ($fixtureDescendant.HasExited) { throw "Fixture child exited before observation" }
            } elseif ($fixtureRecord.pid -ne $fixtureProcess.Id) {
                throw "Fixture readiness did not identify the sentinel"
            }
        }
        if (-not $fixtureProcess.HasExited) { $fixtureProcess.Kill($true) }
        if (-not $fixtureProcess.WaitForExit(5000)) { throw "Owned fixture did not stop" }
        if ($fixtureDescendant -and -not $fixtureDescendant.WaitForExit(5000)) { throw "Owned fixture descendant did not stop" }
        return [pscustomobject]@{
            Role = $Role
            Ready = $fixtureOk
            PreserveSystemRoot = $PreserveSystemRoot
            ExitCode = $fixtureProcess.ExitCode
            Stderr = $fixtureError.GetAwaiter().GetResult()
            Stdout = $fixtureOutput.GetAwaiter().GetResult()
        }
    } finally {
        if ($fixtureStarted -and -not $fixtureProcess.HasExited) {
            $fixtureProcess.Kill($true)
            $fixtureProcess.WaitForExit(5000) | Out-Null
        }
        if ($fixtureDescendant) { $fixtureDescendant.Dispose() }
        $fixtureProcess.Dispose()
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}

foreach ($fixtureRole in @("sentinel", "parent")) {
    $fixtureResult = Invoke-FixtureProbe $fixtureRole $false
    $fixtureResult | ConvertTo-Json -Compress | Write-Host
    if (-not $fixtureResult.Ready) {
        # Compare one essential OS variable without inheriting any credentials.
        $fixtureControl = Invoke-FixtureProbe $fixtureRole $true
        $fixtureControl | ConvertTo-Json -Compress | Write-Host
        throw "The current Rust process fixture failed startup; inspect the exit code and stderr above."
    }
}
