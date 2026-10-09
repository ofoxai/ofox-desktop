# Dependency-free regression checks for the Windows verification runner.
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$runner = Join-Path (Split-Path -Parent $PSScriptRoot) "test-windows.ps1"
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($runner, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw ($parseErrors | Out-String) }
$functions = $ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -in @("Resolve-NativeCommand", "Invoke-VerificationStep")
}, $false)
if ($functions.Count -ne 2) { throw "Runner verification functions were not found." }
foreach ($function in $functions) {
    . ([scriptblock]::Create($function.Extent.Text))
}

$reportDirectory = Join-Path ([IO.Path]::GetTempPath()) ("ofox-runner-regression-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $reportDirectory | Out-Null
$steps = New-Object 'System.Collections.Generic.List[object]'
$nativeProgram = (Get-Process -Id $PID).Path
try {
    # Native stderr is not a failed check when the process exits successfully.
    Invoke-VerificationStep "native-progress" $nativeProgram @("-NoProfile", "-Command", '[Console]::Error.WriteLine("normal native progress"); exit 0')
    if ($steps[0].exitCode -ne 0) { throw "Successful native process failed." }
    if ((Get-Content (Join-Path $reportDirectory "native-progress.log") -Raw) -notmatch "normal native progress") {
        throw "Native stderr was not recorded."
    }
    if ($ErrorActionPreference -ne "Stop") { throw "Error preference was not restored." }

    $failed = $false
    try {
        Invoke-VerificationStep "native-failure" $nativeProgram @("-NoProfile", "-Command", "exit 7")
    } catch { $failed = $true }
    if (-not $failed -or $steps[1].exitCode -ne 7) { throw "A failing native process was reported as passing." }
    if ($ErrorActionPreference -ne "Stop") { throw "Error preference was not restored after failure." }

    # A missing executable must not reuse the last successful exit status.
    Invoke-VerificationStep "native-success" $nativeProgram @("-NoProfile", "-Command", "exit 0")
    $failed = $false
    try {
        Invoke-VerificationStep "missing-executable" (Join-Path $reportDirectory "missing.exe") @()
    } catch { $failed = $true }
    if (-not $failed) { throw "A missing executable was reported as passing." }

    $failed = $false
    try { Resolve-NativeCommand "ofox-nonexistent-fixture-command" } catch { $failed = $true }
    if (-not $failed) { throw "Missing prerequisite was not rejected." }
    Write-Host "Windows runner regressions passed."
} finally {
    Remove-Item -Recurse -Force $reportDirectory
}
