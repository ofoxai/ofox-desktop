# Development-only, before any debug target exists. The caller supplies only the
# signed SDK tool and fixed System32 image; no native output leaves this helper.
function Get-DocumentSymbolArguments([string]$Image, [string]$Cache) {
  return @('/op','/pt',$Image,'/s',"SRV*$Cache*https://msdl.microsoft.com/download/symbols")
}

function Invoke-DocumentSymbolCheck($Start, $Control, [hashtable]$State, [int]$TimeoutMs = 30000, [string]$Image = 'ntdll.dll') {
  $process = [Diagnostics.Process]::new(); $process.StartInfo = $Start
  $output = [IO.MemoryStream]::new()
  try {
    if ($Control.IsCompleted) { throw 'symbol_cancelled' }
    if (-not $process.Start()) { throw 'symbol_check_failed' }
    $State.started = $true; $State.closed = $false
    $process.StandardInput.Dispose()
    $streams = @(
      @{ stream = $process.StandardOutput.BaseStream; buffer = [byte[]]::new(4096); task = $null; done = $false; lineBytes = 0 },
      @{ stream = $process.StandardError.BaseStream; buffer = [byte[]]::new(4096); task = $null; done = $false; lineBytes = 0 }
    )
    foreach ($s in $streams) { $s.task = $s.stream.ReadAsync($s.buffer, 0, $s.buffer.Length) }
    $clock = [Diagnostics.Stopwatch]::StartNew(); $bytes = 0
    while (-not ($process.HasExited -and $streams[0].done -and $streams[1].done)) {
      if ($Control.IsCompleted) { throw 'symbol_cancelled' }
      if ($clock.ElapsedMilliseconds -gt $TimeoutMs) { throw 'symbol_deadline' }
      for ($index = 0; $index -lt 2; $index++) {
        $s = $streams[$index]
        if ($s.done -or -not $s.task.IsCompleted) { continue }
        $count = $s.task.GetAwaiter().GetResult()
        if ($count -eq 0) { $s.done = $true; continue }
        $bytes += $count
        if ($bytes -gt 65536) { throw 'symbol_output_bound' }
        for ($i = 0; $i -lt $count; $i++) {
          if ($s.buffer[$i] -eq 10) { $s.lineBytes = 0 } else { $s.lineBytes++ }
          if ($s.lineBytes -gt 4096) { throw 'symbol_output_bound' }
        }
        if ($index -eq 0) { $output.Write($s.buffer, 0, $count) }
        $s.task = $s.stream.ReadAsync($s.buffer, 0, $s.buffer.Length)
      }
      Start-Sleep -Milliseconds 5
    }
    $State.exitCode = $process.ExitCode
    if ($State.exitCode -ne 0) { throw 'symbol_check_failed' }
    $failed = @(); $passed = @(); $single = 0
    $expected = '^\s*SYMCHK:\s+(?:ntdll\.dll|' + [regex]::Escape($Image) + ')\s+PASSED\s*$'
    foreach ($line in ([Text.UTF8Encoding]::new($false, $true).GetString($output.ToArray()) -split '\r?\n')) {
      if ($line -match '^\s*SYMCHK:\s+FAILED files\s*=\s*([0-9]{1,6})\s*$') { $failed += [int]$Matches[1] }
      if ($line -match '^\s*SYMCHK:\s+PASSED \+ IGNORED files\s*=\s*([0-9]{1,6})\s*$') { $passed += [int]$Matches[1] }
      if ($line -match $expected) { $single++ }
    }
    if ($failed.Count -ne 1 -or $passed.Count -ne 1 -or $failed[0] -ne 0 -or $passed[0] -ne 1 -or $single -ne 1) { throw 'symbol_check_incomplete' }
    $State.failedFiles = 0; $State.passedOrIgnoredFiles = 1; $State.passedFile = $true
  } finally {
    try {
      if ($State.started) {
        if (-not $process.HasExited) { $process.Kill($true) }
        if (-not $process.WaitForExit(5000)) { throw 'symbol_cleanup_unconfirmed' }
        $State.exitCode = $process.ExitCode
        $process.StandardInput.Dispose(); $process.StandardOutput.Dispose(); $process.StandardError.Dispose()
        $State.closed = $true
      }
    } finally { $output.Dispose(); $process.Dispose() }
  }
}
