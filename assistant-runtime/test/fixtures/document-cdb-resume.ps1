param([Parameter(Mandatory = $true)][string]$Directory, [Parameter(Mandatory = $true)][string]$Mode)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-engine-stdio.ps1')
if ($Mode -eq 'validate') {
  $rows = @()
  foreach ($kind in @('continue','go')) {
    $base = if ($kind -eq 'continue') { @{childPid=42;primaryTid=43;preSuspendPrevious=1;prePairedResumePrevious=2;continueRequested=$true} }
      else { @{childPid=42;primaryTid=43;hostResumeResult=1;finalSuspendPrevious=1;finalPairedResumePrevious=2} }
    $changes = if ($kind -eq 'continue') { @(@{},@{childPid=44},@{preSuspendPrevious=$true},@{prePairedResumePrevious='2'},@{continueRequested=1}) }
      else { @(@{},@{primaryTid=44},@{hostResumeResult=$true},@{hostResumeResult='1'},@{finalSuspendPrevious=2},@{finalPairedResumePrevious=3}) }
    foreach ($change in $changes) {
      $value = $base.Clone(); foreach ($key in $change.Keys) { $value[$key]=$change[$key] }
      $errorCode = $null
      try { Assert-DocumentCdbRecord ($value | ConvertTo-Json | ConvertFrom-Json) $kind 42 43 } catch { $errorCode = $_.Exception.Message }
      $rows += @{kind=$kind;failure=$errorCode;valid=($change.Count -eq 0)}
    }
  }
  [Console]::Out.WriteLine(($rows | ConvertTo-Json -Compress)); exit
}
$script:drains = 0
$control = [Threading.Tasks.TaskCompletionSource[string]]::new()
$debugger = @{ HasExited = $Mode -in @('late-go','go-abort','closed-continue') }
$name = if ($Mode -in @('late-go','go-abort','cancel-go')) { 'go.json' } else { 'continue.json' }
$child = $null; $failure = $null
try {
  if ($Mode -in @('go-abort','continue-abort')) {
    [IO.File]::WriteAllText((Join-Path $Directory $name), '{"hostResumeResult":1}')
    [IO.File]::WriteAllText((Join-Path $Directory 'abort.json'), '{}')
    $control.SetResult('STOP')
  }
  if ($Mode -eq 'cancel-go') { $control.SetResult('STOP') }
  if ($Mode -eq 'late-go') {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = (Get-Process -Id $PID).Path; $start.UseShellExecute = $false
    foreach ($argument in @('-NoLogo','-NoProfile','-NonInteractive','-File',
      (Join-Path $PSScriptRoot 'document-cdb-resume-writer.ps1'), $Directory)) { $start.ArgumentList.Add($argument) }
    $child = [Diagnostics.Process]::Start($start)
  }
  Wait-DocumentCdbRecord $Directory $name $control.Task ([DateTime]::UtcNow.AddMilliseconds($(if ($Mode -eq 'deadline') { 100 } else { 5000 }))) $debugger { $script:drains++ }
} catch { $failure = $_.Exception.Message }
finally {
  if ($null -ne $child) { if (-not $child.WaitForExit(5000)) { $child.Kill($true); if (-not $child.WaitForExit(5000)) { throw 'writer_cleanup_unconfirmed' } }; $child.Dispose() }
}
[Console]::Out.WriteLine((@{ failure = $failure; debuggerExited = $debugger.HasExited; drains = $script:drains } | ConvertTo-Json -Compress))
