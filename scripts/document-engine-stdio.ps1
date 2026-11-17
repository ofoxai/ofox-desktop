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
