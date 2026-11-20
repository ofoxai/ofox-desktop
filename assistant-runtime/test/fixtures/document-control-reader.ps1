param(
  [ValidateSet('receive', 'cancel', 'deadline')][string]$Mode,
  [string]$SignalDirectory
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-engine-stdio.ps1')
function Send($Value) {
  [Console]::Out.WriteLine(($Value | ConvertTo-Json -Compress))
  [Console]::Out.Flush()
}
$reader = New-DocumentControlReader
try {
  Send @{ event = 'before-read' }
  $inputTask = $reader.ReadLineAsync()
  Send @{ event = 'scheduled'; pending = -not $inputTask.IsCompleted }
  if ($Mode -eq 'receive') {
    if (-not $inputTask.Wait(5000) -or $inputTask.Result -ne 'STOP') { throw 'first-command' }
    Send @{ event = 'stopped' }
    $inputTask = $reader.ReadLineAsync()
    if (-not $inputTask.Wait(5000) -or $inputTask.Result -ne 'CLEAN') { throw 'second-command' }
    Send @{ event = 'cleaned' }
    $inputTask = $reader.ReadLineAsync()
    if (-not $inputTask.Wait(5000) -or $null -ne $inputTask.Result) { throw 'eof' }
    Send @{ event = 'eof' }
  } else {
    $until = [DateTime]::UtcNow.AddMilliseconds($(if ($Mode -eq 'cancel') { 5000 } else { 200 }))
    while (-not $inputTask.IsCompleted) {
      if ([IO.File]::Exists((Join-Path $SignalDirectory 'abort'))) {
        Send @{ event = 'cancelled'; inputPending = $true }
        break
      }
      if ([DateTime]::UtcNow -ge $until) {
        Send @{ event = 'deadline'; inputPending = $true }
        break
      }
      Start-Sleep -Milliseconds 2
    }
  }
} finally {
  $reader.Dispose()
}
