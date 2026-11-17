# One reader owns the control pipe for the controller's entire lifetime.
function New-DocumentControlReader {
  # Console.In is a synchronized TextReader: its ReadLineAsync blocks before
  # returning a Task. Keep an ordinary StreamReader so deadline/abort polling
  # can run while stdin is silent. Reuse it to preserve prefetched commands.
  return [IO.StreamReader]::new(
    [Console]::OpenStandardInput(), [Text.UTF8Encoding]::new($false, $true),
    $false, 1024, $false)
}

function Get-DocumentCdbArguments([uint32]$TargetPid, [string]$Symbols, [string]$Commands) {
  # Match the documented CDB grammar: all startup options before the target.
  # -pb avoids a break-in thread. Request a stop on the replayed ntdll load event
  # of waiting for a breakpoint that would require running the held target.
  return @('-pb','-xe','ld:ntdll.dll','-hd','-sins','-netsyms:no','-noshell','-nosqm','-noinh','-G','-y',$Symbols,'-cf',$Commands,'-p',$TargetPid.ToString())
}

function Wait-DocumentCdbRecord([string]$Directory, [ValidateSet('continue.json','go.json')][string]$Name, $Control, [DateTime]$Deadline, $Debugger, $Drain = $null) {
  $file = Join-Path $Directory $Name
  while ($true) {
    # GO only confirms an already completed Host Resume. An immediate target
    # exit/abort may overtake our observation of that atomic record. Consuming
    # it never authorizes another g; callers still validate its exact identity.
    if ($Name -eq 'go.json' -and (Test-Path -LiteralPath $file)) { return }
    if ($Control.IsCompleted -or (Test-Path -LiteralPath (Join-Path $Directory 'abort.json'))) { throw 'cancelled' }
    if ([DateTime]::UtcNow -ge $Deadline) { throw 'deadline' }
    if ($Name -eq 'continue.json' -and $Debugger.HasExited) { throw 'debugger_exited' }
    if (Test-Path -LiteralPath $file) { return }
    if ($null -ne $Drain) { & $Drain }
    Start-Sleep -Milliseconds 5
  }
}

function Assert-DocumentCdbRecord($Value, [ValidateSet('continue','go')][string]$Kind, [uint32]$ExpectedPid, [uint32]$ExpectedTid) {
  $failure = if ($Kind -eq 'continue') { 'continue_mismatch' } else { 'host_resume_mismatch' }
  $fields = if ($Kind -eq 'continue') { @('childPid','primaryTid','preSuspendPrevious','prePairedResumePrevious') }
    else { @('childPid','primaryTid','hostResumeResult','finalSuspendPrevious','finalPairedResumePrevious') }
  foreach ($key in $fields) { if ($Value.$key -isnot [int] -and $Value.$key -isnot [long]) { throw $failure } }
  if ($Value.childPid -ne $ExpectedPid -or $Value.primaryTid -ne $ExpectedTid) { throw $failure }
  if ($Kind -eq 'continue') {
    if ($Value.preSuspendPrevious -ne 1 -or $Value.prePairedResumePrevious -ne 2 -or
        $Value.continueRequested -isnot [bool] -or -not $Value.continueRequested) { throw $failure }
  } elseif ($Value.hostResumeResult -ne 1 -or $Value.finalSuspendPrevious -ne 1 -or $Value.finalPairedResumePrevious -ne 2) { throw $failure }
}
