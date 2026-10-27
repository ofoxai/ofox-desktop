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
    # Windows PowerShell 5.1 strips embedded double quotes from native arguments.
    # Encode the fixture so this check only exercises stderr and exit handling.
    $progressCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('[Console]::Error.WriteLine("normal native progress"); exit 0'))
    Invoke-VerificationStep "native-progress" $nativeProgram @("-NoProfile", "-EncodedCommand", $progressCommand)
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
    # Both full and -BackendOnly verification must prepare packaged resources
    # before the first Cargo check. Checking direct try-body statements excludes
    # a helper accidentally hidden inside the frontend-only branch.
    $tryBody = $ast.Find({
        param($node)
        $node -is [Management.Automation.Language.TryStatementAst] -and
            $node.Body.Extent.Text -match 'runner-self-tests'
    }, $false).Body.Statements
    $prepareIndex = -1
    $rustIndex = -1
    for ($index = 0; $index -lt $tryBody.Count; $index++) {
        $statement = $tryBody[$index]
        if ($statement -is [Management.Automation.Language.PipelineAst] -and
            $statement.Extent.Text -match '^Initialize-AssistantRuntime\s*$') {
            $prepareIndex = $index
        }
        if ($statement.Extent.Text -match '^Invoke-VerificationStep "rust-format"') { $rustIndex = $index }
    }
    if ($prepareIndex -lt 0 -or $prepareIndex -ge $rustIndex) {
        throw "Assistant preparation must execute unconditionally before Rust checks, including -BackendOnly."
    }
    $prepareFunction = $ast.Find({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
            $node.Name -eq "Initialize-AssistantRuntime"
    }, $false)
    if (-not $prepareFunction) { throw "Assistant preparation function is missing." }
    . ([scriptblock]::Create($prepareFunction.Extent.Text))
    $preparationCalls = New-Object 'System.Collections.Generic.List[object]'
    function Resolve-NativeCommand([string]$Name) {
        if ($Name -ne "node") { throw "Backend preparation unexpectedly requires $Name." }
        return "fixture-node.exe"
    }
    function Invoke-VerificationStep([string]$Name, [string]$Program, [string[]]$Arguments) {
        $preparationCalls.Add([pscustomobject]@{ name = $Name; program = $Program; arguments = $Arguments })
    }
    $repoRoot = Join-Path $reportDirectory "checkout with spaces"
    Initialize-AssistantRuntime
    if (($preparationCalls.name -join ',') -ne 'assistant-node-version,assistant-runtime-prepare,assistant-production-smoke') {
        throw "Assistant resources must be prepared before their production smoke check."
    }
    if ($preparationCalls.Where({ $_.program -ne 'fixture-node.exe' }).Count -ne 0) {
        throw "Preparation did not use the resolved Node executable."
    }
    if ($preparationCalls[1].arguments.Count -ne 1 -or $preparationCalls[1].arguments[0] -ne (Join-Path $repoRoot 'scripts/assistant-prepare.mjs')) {
        throw "Preparation must pass the script path as one argument, including spaces."
    }
    if ($preparationCalls[2].arguments.Count -ne 1 -or $preparationCalls[2].arguments[0] -ne (Join-Path $repoRoot 'scripts/assistant-smoke.mjs')) {
        throw "Preparation must verify the copied production tree."
    }
    Write-Host "Windows runner regressions passed."
} finally {
    Remove-Item -Recurse -Force $reportDirectory
}
