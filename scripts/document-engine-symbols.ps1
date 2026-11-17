# Development-only, before any debug target exists. The caller supplies only the
# signed SDK tool and fixed System32 image; no native output leaves this helper.
function Get-DocumentSymbolArguments([string]$Image, [string]$Cache) {
  return @('/op','/pt',$Image,'/s',"SRV*$Cache*https://msdl.microsoft.com/download/symbols")
}

function Get-DocumentSymbolOutputShape([byte[]]$Bytes, [string]$Image) {
  $shape = @{ capturedBytes = $Bytes.Length; bom = 'none'; nulEvenBytes = 0; nulOddBytes = 0; crBytes = 0; lfBytes = 0
    individualPassLines = 0; failedTotalLines = 0; passedTotalLines = 0; failedFiles = $null; passedOrIgnoredFiles = $null; usageMention = $false; tokenRows = @() }
  if ($Bytes.Length -ge 2 -and $Bytes[0] -eq 255 -and $Bytes[1] -eq 254) { $shape.bom = 'utf16le' }
  elseif ($Bytes.Length -ge 2 -and $Bytes[0] -eq 254 -and $Bytes[1] -eq 255) { $shape.bom = 'utf16be' }
  elseif ($Bytes.Length -ge 3 -and $Bytes[0] -eq 239 -and $Bytes[1] -eq 187 -and $Bytes[2] -eq 191) { $shape.bom = 'utf8' }
  for ($i = 0; $i -lt $Bytes.Length; $i++) {
    if ($Bytes[$i] -eq 0) { if ($i % 2 -eq 0) { $shape.nulEvenBytes++ } else { $shape.nulOddBytes++ } }
    if ($Bytes[$i] -eq 13) { $shape.crBytes++ }; if ($Bytes[$i] -eq 10) { $shape.lfBytes++ }
  }
  $allowed = @('SYMCHK','SYMSRV','DBGHELP','PASSED','FAILED','IGNORED','FILES','NTDLL.DLL','NTDLL.PDB','USAGE',':','=','+')
  $single = '^\s*SYMCHK:\s+(?:ntdll\.dll|' + [regex]::Escape($Image) + ')\s+PASSED\s*$'
  $failed = @(); $passed = @()
  # This replacement decoder is for shape only. The verifier below retains its
  # strict decoder and original stdout-only grammar until native evidence exists.
  foreach ($line in ([Text.Encoding]::UTF8.GetString($Bytes) -split '\r?\n')) {
    if ($line -match $single) { $shape.individualPassLines++ }
    if ($line -match '^\s*SYMCHK:\s+FAILED files\s*=\s*([0-9]{1,6})\s*$') { $failed += [int]$Matches[1] }
    if ($line -match '^\s*SYMCHK:\s+PASSED \+ IGNORED files\s*=\s*([0-9]{1,6})\s*$') { $passed += [int]$Matches[1] }
    if ($line -match '(?i)\busage\b') { $shape.usageMention = $true }
    if ($line.Trim().Length -eq 0 -or $shape.tokenRows.Count -ge 8) { continue }
    $row = @()
    foreach ($match in [regex]::Matches($line, '[A-Za-z_][A-Za-z0-9_.-]*|[:=+]|[^\s]')) {
      $token = $match.Value.ToUpperInvariant()
      if ($allowed -notcontains $token) { $token = '?' }
      if ($token -ne '?' -or $row.Count -eq 0 -or $row[-1] -ne '?') { $row += $token }
      if ($row.Count -ge 16) { break }
    }
    $shape.tokenRows += ,$row
  }
  $shape.failedTotalLines = $failed.Count; $shape.passedTotalLines = $passed.Count
  if ($failed.Count -eq 1) { $shape.failedFiles = $failed[0] }
  if ($passed.Count -eq 1) { $shape.passedOrIgnoredFiles = $passed[0] }
  return $shape
}

function Invoke-DocumentSymbolCheck($Start, $Control, [hashtable]$State, [int]$TimeoutMs = 30000, [string]$Image = 'ntdll.dll') {
  $process = [Diagnostics.Process]::new(); $process.StartInfo = $Start
  $output = [IO.MemoryStream]::new(); $errorOutput = [IO.MemoryStream]::new()
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
        else { $errorOutput.Write($s.buffer, 0, $count) }
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
    if ($failed.Count -eq 1) { $State.failedFiles = $failed[0] }
    if ($passed.Count -eq 1) { $State.passedOrIgnoredFiles = $passed[0] }
    if ($failed.Count -ne 1 -or $passed.Count -ne 1 -or $failed[0] -ne 0 -or $passed[0] -ne 1 -or $single -ne 1) { throw 'symbol_check_incomplete' }
    $State.failedFiles = 0; $State.passedOrIgnoredFiles = 1; $State.passedFile = $true
  } finally {
    try { $State.outputShape = @{ stdout = (Get-DocumentSymbolOutputShape $output.ToArray() $Image); stderr = (Get-DocumentSymbolOutputShape $errorOutput.ToArray() $Image) } } catch { }
    try {
      if ($State.started) {
        if (-not $process.HasExited) { $process.Kill($true) }
        if (-not $process.WaitForExit(5000)) { throw 'symbol_cleanup_unconfirmed' }
        $State.exitCode = $process.ExitCode
        $process.StandardInput.Dispose(); $process.StandardOutput.Dispose(); $process.StandardError.Dispose()
        $State.closed = $true
      }
    } finally { $output.Dispose(); $errorOutput.Dispose(); $process.Dispose() }
  }
}
