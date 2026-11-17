param([Parameter(Mandatory = $true)][string]$RawDirectory)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
. (Join-Path $PSScriptRoot 'document-engine-stdio.ps1')
. (Join-Path $PSScriptRoot 'document-engine-symbols.ps1')
. (Join-Path $PSScriptRoot 'document-engine-operations.ps1')
$controlReader = New-DocumentControlReader
$ownsRaw = $false; $debugger = $null; $writer = $null; $metadata = $null
$stage = 'controller_setup'; $debuggerStarted = $false; $debuggerClosed = $true
$outTask = $null; $errTask = $null; $outDone = $false; $errDone = $false
$targetSeen = $false; $targetPid = $null; $targetTid = $null
$before = $null; $after = $null; $section = ''; $tail = $false
$capturedBytes = 0; $stdoutLines = 0; $stderrLines = 0
$setupStep = 0; $firstFailureStage = $null; $cleanupFailure = $null; $debuggerExited = $null; $debuggerExitCode = $null
$promptLines = 0; $markerMentionLines = 0; $numericTargetLines = 0; $gflagValueLines = 0; $nullCharacters = 0
$reportedWin32Errors = @()
$symbolState = @{ started = $false; closed = $true; exitCode = $null; failedFiles = $null; passedOrIgnoredFiles = $null; passedFile = $false }
$gflagHints = @{ symbols = $false; types = $false; peb = $false; globalFlags = $false; failure = $false; extension = $false }
$continueSent = $false; $hostResumeSeen = $false
$operationSetup = $null

function Send-Protocol($Value) { [Console]::Out.WriteLine(($Value | ConvertTo-Json -Depth 12 -Compress)); [Console]::Out.Flush() }
function Write-Json([string]$Name, $Value) {
  $temporary = Join-Path $RawDirectory ([Guid]::NewGuid().ToString('N') + '.tmp')
  [IO.File]::WriteAllText($temporary, ($Value | ConvertTo-Json -Depth 12 -Compress), [Text.UTF8Encoding]::new($false))
  [IO.File]::Move($temporary, (Join-Path $RawDirectory $Name), $true)
}
function Read-Json([string]$Name) {
  $file = Join-Path $RawDirectory $Name
  if ((Get-Item -LiteralPath $file).Length -gt 32768) { throw 'input_bound' }
  return ([IO.File]::ReadAllText($file) | ConvertFrom-Json)
}
function Find-DebuggerTool([ValidateSet('cdb','symchk')][string]$Name) {
  $file = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\Debuggers\x64\$Name.exe"
  if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw 'debugger_unavailable' }
  $signature = Get-AuthenticodeSignature -LiteralPath $file
  if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)') { throw 'debugger_untrusted' }
  $version = (Get-Item -LiteralPath $file).VersionInfo
  return @{ path = $file; safe = @{ available = $true; signatureValid = $true; microsoftSigned = $true
    version = "$($version.FileMajorPart).$($version.FileMinorPart).$($version.FileBuildPart).$($version.FilePrivatePart)"
    sha256 = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() } }
}
function Read-CdbLine([string]$Line, [bool]$Stderr) {
  $script:capturedBytes += [Text.Encoding]::UTF8.GetByteCount($Line) + 1
  if ($Line.Length -gt 16384 -or $script:capturedBytes -gt 4194304) { throw 'output_bound' }
  $script:writer.WriteLine($Line)
  if ($null -ne $script:operationSetup) { Read-DocumentOperationSetupLine $script:operationSetup $Line }
  if ($Stderr) { $script:stderrLines++ } else { $script:stdoutLines++ }
  # Shape only: these counts never authorize READY and disclose no raw text.
  if ($Line -match '^(?:\d{1,3}:)?\d{1,3}:[a-fA-F0-9]{3}>') { $script:promptLines++ }
  if ($Line.Contains('OFOX_CDB_RAW_V1')) { $script:markerMentionLines++ }
  if ($Line -match 'OFOX_CDB_RAW_V1 TARGET [a-fA-F0-9]{1,8} [a-fA-F0-9]{1,8} END') { $script:numericTargetLines++ }
  if ($Line -match '^\s*(?:Current|New) NtGlobalFlag contents:\s*(?:0x)?[a-fA-F0-9]{1,8}\s*$') { $script:gflagValueLines++ }
  # Lexical observations only, scoped to the two fixed gflag commands. They
  # disclose no text and never establish a root cause or authorize READY.
  if ($script:section -in @('before', 'after')) {
    if ($Line -match '(?i)\bsymbols?\b|\bntdll!') { $script:gflagHints.symbols = $true }
    if ($Line -match '(?i)\btypes?\b|GetField(?:Value|Offset)') { $script:gflagHints.types = $true }
    if ($Line -match '(?i)\b_?PEB\b') { $script:gflagHints.peb = $true }
    if ($Line -match '(?i)\bNtGlobalFlag\b') { $script:gflagHints.globalFlags = $true }
    if ($Line -match '(?i)\b(?:error|failed|failure|unable|cannot|could not|couldn''t|not found|unresolved)\b') { $script:gflagHints.failure = $true }
    if ($Line -match '(?i)\bextensions?\b') { $script:gflagHints.extension = $true }
  }
  $script:nullCharacters += $Line.Length - $Line.Replace([string][char]0, '').Length
  if ($script:reportedWin32Errors.Count -lt 8 -and $Line -match '\bWin32 error (?:0n([0-9]{1,10})|0x([a-fA-F0-9]{1,8}))\b') {
    $value = if ($Matches[1]) { [uint64]::Parse($Matches[1]) } else { [Convert]::ToUInt64($Matches[2], 16) }
    if ($value -le [uint32]::MaxValue) { $script:reportedWin32Errors += [uint32]$value }
  }
  if ($Line -match '^OFOX_CDB_RAW_V1 TARGET ([a-fA-F0-9]{1,8}) ([a-fA-F0-9]{1,8}) END$') {
    if ($script:targetSeen -or $script:setupStep -ne 0) { throw 'target_mismatch' }
    $script:targetSeen = $true
    $script:targetPid = [Convert]::ToUInt32($Matches[1], 16)
    $script:targetTid = [Convert]::ToUInt32($Matches[2], 16)
    $script:setupStep = 1
  } elseif ($Line -eq 'OFOX_CDB_RAW_V1 BEFORE_BEGIN') { if ($script:setupStep -ne 1) { throw 'gflag_before_unreadable' }; $script:section = 'before'; $script:setupStep = 2
  } elseif ($Line -eq 'OFOX_CDB_RAW_V1 BEFORE_END') { if ($script:setupStep -ne 2 -or $null -eq $script:before) { throw 'gflag_before_unreadable' }; $script:section = ''; $script:setupStep = 3
  } elseif ($Line -eq 'OFOX_CDB_RAW_V1 AFTER_BEGIN') { if ($script:setupStep -ne 3) { throw 'gflag_after_unreadable' }; $script:section = 'after'; $script:setupStep = 4
  } elseif ($Line -eq 'OFOX_CDB_RAW_V1 AFTER_END') { if ($script:setupStep -ne 4 -or $null -eq $script:after) { throw 'gflag_after_unreadable' }; $script:section = ''; $script:setupStep = 5
  } elseif ($Line -eq 'OFOX_CDB_RAW_V1 CONFIG_END') { if ($script:setupStep -ne 5) { throw 'init_tail_missing' }; $script:tail = $true; $script:setupStep = 6
  } elseif ($Line -match '^\s*(?:Current|New) NtGlobalFlag contents:\s*(?:0x)?([a-fA-F0-9]{1,8})\s*$') {
    $number = [Convert]::ToUInt32($Matches[1], 16)
    if ($script:section -eq 'before') { if ($null -ne $script:before) { throw 'gflag_before_unreadable' }; $script:before = $number }
    if ($script:section -eq 'after') { if ($null -ne $script:after) { throw 'gflag_after_unreadable' }; $script:after = $number }
  }
}
function Drain-Cdb([bool]$Discard = $false) {
  foreach ($stream in @('out', 'err')) {
    for ($i = 0; $i -lt 500; $i++) {
      $task = if ($stream -eq 'out') { $script:outTask } else { $script:errTask }
      $done = if ($stream -eq 'out') { $script:outDone } else { $script:errDone }
      if ($done -or $null -eq $task -or -not $task.IsCompleted) { break }
      $line = $task.GetAwaiter().GetResult()
      if ($null -eq $line) {
        if ($stream -eq 'out') { $script:outDone = $true } else { $script:errDone = $true }
        break
      }
      if ($stream -eq 'out') { $script:outTask = $script:debugger.StandardOutput.ReadLineAsync() }
      else { $script:errTask = $script:debugger.StandardError.ReadLineAsync() }
      if (-not $Discard) { Read-CdbLine $line ($stream -eq 'err') }
    }
  }
  $script:writer.Flush()
}
function Stop-Cdb {
  if ($null -eq $script:debugger) { return }
  if ($script:debuggerClosed) { return }
  if (-not $script:debugger.HasExited) { $script:debugger.Kill($true) }
  if (-not $script:debugger.WaitForExit(5000)) { throw 'debugger_stop_unconfirmed' }
  $script:debuggerExited = $true
  $script:debuggerExitCode = $script:debugger.ExitCode
  $until = [DateTime]::UtcNow.AddSeconds(5)
  while (-not ($script:outDone -and $script:errDone) -and [DateTime]::UtcNow -lt $until) { Drain-Cdb ($null -ne $script:firstFailureStage); Start-Sleep -Milliseconds 5 }
  if (-not ($script:outDone -and $script:errDone)) { throw 'debugger_drain_unconfirmed' }
  $script:debuggerClosed = $true
  $script:debugger.StandardInput.Dispose(); $script:debugger.StandardOutput.Dispose(); $script:debugger.StandardError.Dispose()
}

try {
  if (-not $IsWindows -or -not [Environment]::Is64BitProcess) { throw 'native_required' }
  $runner = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd([IO.Path]::DirectorySeparatorChar)
  $RawDirectory = [IO.Path]::GetFullPath($RawDirectory)
  if (-not $RawDirectory.StartsWith(($runner + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase) -or
      [IO.Path]::GetFileName($RawDirectory) -notmatch '^ofox-document-etw-[a-f0-9-]{36}$') { throw 'private_directory_invalid' }
  $directory = Get-Item -LiteralPath $RawDirectory
  if (($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or [IO.Directory]::GetFileSystemEntries($RawDirectory).Length -ne 0) { throw 'private_directory_invalid' }
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  try { $sid = $identity.User.Value } finally { $identity.Dispose() }
  $acl = [Security.AccessControl.DirectorySecurity]::new()
  $acl.SetSecurityDescriptorSddlForm("O:${sid}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;${sid})")
  Set-Acl -LiteralPath $RawDirectory -AclObject $acl
  $ownsRaw = $true
  $stage = 'debugger_inventory'
  $cdb = Find-DebuggerTool 'cdb'
  $stage = 'symbol_inventory'
  $symchk = Find-DebuggerTool 'symchk'
  $stage = 'native_controller_compile'
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class DocumentCdbNative {
  [StructLayout(LayoutKind.Sequential)] struct FileInfo {
    public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Creation, Access, Write;
    public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
  }
  [StructLayout(LayoutKind.Sequential)] struct Basic {
    public long ProcessTime, JobTime; public uint Flags; public UIntPtr Minimum, Maximum;
    public uint Active; public UIntPtr Affinity; public uint Priority, Scheduling;
  }
  [StructLayout(LayoutKind.Sequential)] struct Counters { public ulong A,B,C,D,E,F; }
  [StructLayout(LayoutKind.Sequential)] struct Limits {
    public Basic Basic; public Counters Counters; public UIntPtr ProcessMemory, JobMemory, PeakProcess, PeakJob;
  }
  static IntPtr ownedJob;
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern Microsoft.Win32.SafeHandles.SafeFileHandle CreateFileW(string file, uint access, uint sharing, IntPtr security, uint creation, uint flags, IntPtr template);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetFileInformationByHandle(Microsoft.Win32.SafeHandles.SafeFileHandle file, out FileInfo info);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateJobObjectW(IntPtr attributes, string name);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job, int kind, ref Limits limits, uint bytes);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
  [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern uint QueryDosDeviceW(string drive, StringBuilder value, uint size);
  public static void OwnControllerJob() {
    ownedJob = CreateJobObjectW(IntPtr.Zero, null);
    var limits = new Limits(); limits.Basic.Flags = 0x2000;
    if (ownedJob == IntPtr.Zero || !SetInformationJobObject(ownedJob, 9, ref limits, (uint)Marshal.SizeOf<Limits>()) ||
        !AssignProcessToJobObject(ownedJob, GetCurrentProcess())) throw new InvalidOperationException("job_setup");
    // Keep this raw handle open until controller process exit. Its children
    // inherit the Job; unexpected controller exit kills its owned debugger.
  }
  public static void CopySymbol(string source, string destination) {
    using (var handle = CreateFileW(source, 0x80000000, 1, IntPtr.Zero, 3, 0x00200000, IntPtr.Zero)) {
      FileInfo info;
      if (handle.IsInvalid || !GetFileInformationByHandle(handle, out info) || info.Links != 1 ||
          (info.Attributes & 0x410) != 0 || info.SizeHigh != 0 || info.SizeLow == 0 || info.SizeLow > 67108864)
        throw new InvalidOperationException("symbol_cache_invalid");
      using (var input = new System.IO.FileStream(handle, System.IO.FileAccess.Read))
      using (var output = new System.IO.FileStream(destination, System.IO.FileMode.CreateNew, System.IO.FileAccess.Write, System.IO.FileShare.None)) input.CopyTo(output);
    }
  }
  public static string[] WindowsAliases(string root) {
    root = System.IO.Path.GetFullPath(root).TrimEnd('\\');
    var output = new System.Collections.Generic.List<string> { root, @"\??\"+root, @"\\?\"+root };
    var value = new StringBuilder(32768);
    if (root.Length > 2 && root[1] == ':' && QueryDosDeviceW(root.Substring(0,2), value, 32768) != 0) {
      string device = value.ToString().Split('\0')[0];
      if (device.StartsWith(@"\Device\", StringComparison.OrdinalIgnoreCase)) output.Add(device+root.Substring(2));
    }
    return output.ToArray();
  }
}
'@
  $stage = 'controller_job_setup'
  [DocumentCdbNative]::OwnControllerJob()
  $symbols = Join-Path $RawDirectory 'symbols'
  [IO.Directory]::CreateDirectory($symbols) | Out-Null
  $metadata = @{ tools = @{ cdb = $cdb.safe; symchk = $symchk.safe }; mode = 'cdb-loader-snaps'; maximumFileMiB = 4
    userInteractive = [Environment]::UserInteractive; sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
    debuggerJobKillOnClose = $true; symbolPreparation = $symbolState }
  $inputTask = $controlReader.ReadLineAsync()
  $stage = 'symbol_prepare'
  $download = Join-Path $RawDirectory 'symbol-download'
  [IO.Directory]::CreateDirectory($download) | Out-Null
  $image = Join-Path $env:SystemRoot 'System32\ntdll.dll'
  $prepare = [Diagnostics.ProcessStartInfo]::new()
  $prepare.FileName = $symchk.path; $prepare.WorkingDirectory = $RawDirectory; $prepare.UseShellExecute = $false; $prepare.CreateNoWindow = $true
  $prepare.RedirectStandardInput = $true; $prepare.RedirectStandardOutput = $true; $prepare.RedirectStandardError = $true
  $prepare.Environment.Clear()
  $prepare.Environment['SystemRoot'] = $env:SystemRoot; $prepare.Environment['WINDIR'] = $env:SystemRoot
  $prepare.Environment['PATH'] = Join-Path $env:SystemRoot 'System32'
  foreach ($key in @('TMP','TEMP','USERPROFILE','HOME','APPDATA','LOCALAPPDATA')) { $prepare.Environment[$key] = $RawDirectory }
  foreach ($argument in (Get-DocumentSymbolArguments $image $download)) { $prepare.ArgumentList.Add($argument) }
  Invoke-DocumentSymbolCheck $prepare $inputTask $symbolState 30000 $image
  $stage = 'symbol_cache_invalid'
  $pdbs = @(Get-ChildItem -LiteralPath $download -Recurse -File -Filter 'ntdll.pdb')
  if ($pdbs.Count -ne 1) { throw 'symbol_cache_invalid' }
  $ancestor = $pdbs[0].Directory
  while ($null -ne $ancestor -and $ancestor.FullName.StartsWith(($download + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase)) {
    if (($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'symbol_cache_invalid' }
    $ancestor = $ancestor.Parent
  }
  if ($null -eq $ancestor -or $ancestor.FullName -ne $download -or ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'symbol_cache_invalid' }
  [DocumentCdbNative]::CopySymbol($pdbs[0].FullName, (Join-Path $symbols 'ntdll.pdb'))
  if ($inputTask.IsCompleted) { throw 'symbol_cancelled' }
  Write-Json 'roots.json' @{ windows = @([DocumentCdbNative]::WindowsAliases($env:SystemRoot)) }
  $commands = Join-Path $RawDirectory 'init.cmd'
  $initialCommands = @'
.echo OFOX_CDB_RAW_V1 TARGET_BEGIN
.printf "OFOX_CDB_RAW_V1 TARGET %x %x END\n", @$tpid, @$tid
.echo OFOX_CDB_RAW_V1 BEFORE_BEGIN
!gflag
.echo OFOX_CDB_RAW_V1 BEFORE_END
!gflag +sls
.echo OFOX_CDB_RAW_V1 AFTER_BEGIN
!gflag
.echo OFOX_CDB_RAW_V1 AFTER_END
'@
  Write-Json 'controller.json' @{ diagnosticOnly = $true; controllerReady = $true }
  Send-Protocol @{ state = 'started'; metadata = $metadata }
  $stage = 'target_not_seen'
  $deadline = [DateTime]::UtcNow.AddSeconds(900)
  while (-not (Test-Path -LiteralPath (Join-Path $RawDirectory 'target.json'))) {
    if ($inputTask.IsCompleted -or (Test-Path -LiteralPath (Join-Path $RawDirectory 'abort.json'))) { throw 'cancelled' }
    if ([DateTime]::UtcNow -gt $deadline) { throw 'deadline' }
    Start-Sleep -Milliseconds 10
  }
  if ($inputTask.IsCompleted -or (Test-Path -LiteralPath (Join-Path $RawDirectory 'abort.json'))) { throw 'cancelled' }
  $target = Read-Json 'target.json'
  if ($target.childPid -isnot [long] -and $target.childPid -isnot [int]) { throw 'target_mismatch' }
  if ($target.primaryTid -isnot [long] -and $target.primaryTid -isnot [int]) { throw 'target_mismatch' }
  if ($target.childPid -le 0 -or $target.childPid -gt [uint32]::MaxValue -or $target.primaryTid -le 0 -or $target.primaryTid -gt [uint32]::MaxValue) { throw 'target_mismatch' }
  $operationSetup = New-DocumentOperationSetup $target.childPid $target.primaryTid
  Confirm-DocumentOperationArchitecture $operationSetup $target.architecture
  $operationProgram = Get-DocumentOperationProgram $target.primaryTid
  foreach ($file in $operationProgram.files.GetEnumerator()) {
    [IO.File]::WriteAllText((Join-Path $RawDirectory $file.Key), $file.Value, [Text.UTF8Encoding]::new($false))
  }
  [IO.File]::WriteAllText($commands, ($initialCommands + "`n" + ($operationProgram.commands -join "`n") + "`n.echo OFOX_CDB_RAW_V1 CONFIG_END`n"), [Text.UTF8Encoding]::new($false))
  $writer = [IO.StreamWriter]::new((Join-Path $RawDirectory 'debugger.log'), $false, [Text.UTF8Encoding]::new($false))
  $start = [Diagnostics.ProcessStartInfo]::new()
  $start.FileName = $cdb.path; $start.WorkingDirectory = $RawDirectory; $start.UseShellExecute = $false; $start.CreateNoWindow = $true
  $start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
  $start.Environment.Clear()
  $start.Environment['SystemRoot'] = $env:SystemRoot; $start.Environment['WINDIR'] = $env:SystemRoot
  $start.Environment['PATH'] = Join-Path $env:SystemRoot 'System32'
  foreach ($key in @('TMP','TEMP','USERPROFILE','HOME','APPDATA','LOCALAPPDATA')) { $start.Environment[$key] = $RawDirectory }
  foreach ($argument in (Get-DocumentCdbArguments $target.childPid $symbols $commands)) { $start.ArgumentList.Add($argument) }
  $debugger = [Diagnostics.Process]::new(); $debugger.StartInfo = $start
  $stage = 'debugger_attach_start'
  if ($inputTask.IsCompleted -or (Test-Path -LiteralPath (Join-Path $RawDirectory 'abort.json'))) { throw 'cancelled' }
  if (-not $debugger.Start()) { throw 'debugger_exited' }
  $debuggerStarted = $true; $debuggerClosed = $false
  $outTask = $debugger.StandardOutput.ReadLineAsync(); $errTask = $debugger.StandardError.ReadLineAsync()
  $stage = 'debugger_handshake'
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  while (-not $tail) {
    Drain-Cdb
    if (Test-Path -LiteralPath (Join-Path $RawDirectory 'abort.json')) { throw 'cancelled' }
    if ($debugger.HasExited) { Drain-Cdb; if (-not $tail) { throw 'debugger_exited' } }
    if ([DateTime]::UtcNow -gt $deadline) {
      if (-not $targetSeen) { throw 'target_not_seen' }
      if ($null -eq $before) { throw 'gflag_before_unreadable' }
      if ($null -eq $after) { throw 'gflag_after_unreadable' }
      throw 'init_tail_missing'
    }
    Start-Sleep -Milliseconds 5
  }
  if (-not $targetSeen -or $targetPid -ne $target.childPid -or $targetTid -ne $target.primaryTid) { throw 'target_mismatch' }
  if ($null -eq $before) { throw 'gflag_before_unreadable' }
  if ($null -eq $after) { throw 'gflag_after_unreadable' }
  if ($after -ne ($before -bor 2)) { throw 'gflag_changed_unexpectedly' }
  Assert-DocumentOperationSetup $operationSetup
  $writer.WriteLine(('OFOX_NT_V1 INSTALLED pid={0:x} tid={1:x} thread={2} count={3} arch=amd64 abi=windows-x64 END' -f $targetPid,$targetTid,$operationSetup.threadIndex,@(Get-DocumentOperationDefinitions).Count))
  $writer.WriteLine(('OFOX_CDB_V2 BEFORE pid={0:x} tid={1:x} flags={2:x} END' -f $targetPid,$targetTid,$before))
  $writer.WriteLine(('OFOX_CDB_V2 AFTER pid={0:x} tid={1:x} flags={2:x} END' -f $targetPid,$targetTid,$after))
  $writer.WriteLine('OFOX_CDB_V2 READY END'); $writer.Flush()
  Write-Json 'ready.json' @{ childPid = $targetPid; primaryTid = $targetTid; beforeFlags = $before; afterFlags = $after; configConfirmed = $true }
  $stage = 'waiting_continue'
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  Wait-DocumentCdbRecord $RawDirectory 'continue.json' $inputTask $deadline $debugger { Drain-Cdb }
  $permission = Read-Json 'continue.json'
  Assert-DocumentCdbRecord $permission 'continue' $targetPid $targetTid
  Drain-Cdb
  if ($inputTask.IsCompleted -or (Test-Path -LiteralPath (Join-Path $RawDirectory 'abort.json'))) { throw 'cancelled' }
  if ([DateTime]::UtcNow -ge $deadline) { throw 'deadline' }
  if ($debugger.HasExited) { throw 'debugger_exited' }
  $writer.WriteLine(('OFOX_CDB_V2 PRE_ATTACH_HOLD pid={0:x} tid={1:x} suspend=1 paired=2 END' -f $targetPid,$targetTid))
  $writer.WriteLine('OFOX_CDB_V2 CONTINUE_REQUESTED END'); $writer.Flush()
  # Release only the debugger's stop. The separately proven CREATE_SUSPENDED
  # hold stays in force; Rust remeasures exact 1/paired2 before original Resume1.
  if ($inputTask.IsCompleted -or (Test-Path -LiteralPath (Join-Path $RawDirectory 'abort.json'))) { throw 'cancelled' }
  if ([DateTime]::UtcNow -ge $deadline) { throw 'deadline' }
  $debugger.StandardInput.WriteLine('g'); $debugger.StandardInput.Flush()
  $continueSent = $true
  $stage = 'waiting_host_resume'
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  # Continue bounded private draining to avoid pipe backpressure. Output or CDB
  # exit can precede GO observation; the projector requires the complete global
  # Host proof, never marker position alone. This stage never sends a second g.
  Wait-DocumentCdbRecord $RawDirectory 'go.json' $inputTask $deadline $debugger { Drain-Cdb }
  $go = Read-Json 'go.json'
  Assert-DocumentCdbRecord $go 'go' $targetPid $targetTid
  $hostResumeSeen = $true
  $writer.WriteLine(('OFOX_CDB_V2 HOST_RESUMED pid={0:x} tid={1:x} hold=1 paired=2 previous=1 END' -f $targetPid,$targetTid)); $writer.Flush()
  $stage = 'running_diagnostic'
  $deadline = [DateTime]::UtcNow.AddSeconds(35)
  while (-not $debugger.HasExited) {
    Drain-Cdb
    if (Test-Path -LiteralPath (Join-Path $RawDirectory 'abort.json')) { break }
    if ([DateTime]::UtcNow -gt $deadline) { throw 'deadline' }
    Start-Sleep -Milliseconds 5
  }
  Stop-Cdb
  $stage = 'debugger_closed'
  Write-Json 'finished.json' @{ stage = $stage; debuggerClosed = $debuggerClosed }
  $writer.Dispose(); $writer = $null
  Write-Json 'debugger-state.json' @{ stage = $stage; debuggerStarted = $debuggerStarted; debuggerClosed = $debuggerClosed
    operationSetup = (Get-DocumentOperationSetupState $operationSetup)
    debuggerExitCode = $debuggerExitCode; debuggerExited = $debuggerExited; stdoutLines = $stdoutLines; stderrLines = $stderrLines; capturedBytes = $capturedBytes
    targetSeen = $targetSeen; beforeReadable = ($null -ne $before); afterReadable = ($null -ne $after); initTailSeen = $tail; continueSent = $continueSent; hostResumeSeen = $hostResumeSeen
    promptLines = $promptLines; markerMentionLines = $markerMentionLines; numericTargetLines = $numericTargetLines; gflagValueLines = $gflagValueLines; nullCharacters = $nullCharacters; reportedWin32Errors = @($reportedWin32Errors); gflagHints = $gflagHints }
  Send-Protocol @{ state = 'raw-ready'; metadata = $metadata }
  if (-not $inputTask.Wait([TimeSpan]::FromSeconds(120)) -or $inputTask.Result -ne 'STOP') { throw 'controller_eof' }
  $inputTask = $controlReader.ReadLineAsync()
  if (-not $inputTask.Wait([TimeSpan]::FromSeconds(120)) -or $inputTask.Result -ne 'CLEAN') { throw 'controller_eof' }
} catch {
  $known = @('symbol_cancelled','symbol_deadline','symbol_check_failed','symbol_check_incomplete','symbol_output_bound','symbol_cleanup_unconfirmed','symbol_cache_invalid',
    'operation_architecture_unverified','operation_layout_unverified','operation_thread_unverified','operation_install_unverified','operation_target_invalid',
    'target_not_seen','target_mismatch','gflag_before_unreadable','gflag_after_unreadable','gflag_changed_unexpectedly',
    'init_tail_missing','debugger_exited','deadline','cancelled','output_bound','continue_mismatch','host_resume_mismatch','debugger_stop_unconfirmed','debugger_drain_unconfirmed',
    'debugger_unavailable','debugger_untrusted','private_directory_invalid','controller_eof','input_bound')
  if ($known -contains $_.Exception.Message) { $stage = $_.Exception.Message }
  $firstFailureStage = $stage
  # Publish the diagnosis before killing CDB can signal the attached target.
  # finished.json still waits for actual owned-debugger cleanup.
  if ($ownsRaw) { try { Write-Json 'failed.json' @{ stage = $firstFailureStage } } catch { } }
  try { Stop-Cdb } catch { $debuggerClosed = $false; $cleanupFailure = 'debugger_cleanup_unconfirmed' }
  if ($null -ne $writer) { $writer.Dispose(); $writer = $null }
  if ($ownsRaw) {
    Write-Json 'failed.json' @{ stage = $firstFailureStage; cleanupFailure = $cleanupFailure }
    Write-Json 'finished.json' @{ stage = $firstFailureStage; debuggerClosed = $debuggerClosed }
    Write-Json 'debugger-state.json' @{ stage = $firstFailureStage; cleanupFailure = $cleanupFailure; debuggerStarted = $debuggerStarted; debuggerClosed = $debuggerClosed; debuggerExited = $debuggerExited; debuggerExitCode = $debuggerExitCode
      operationSetup = (Get-DocumentOperationSetupState $operationSetup)
      stdoutLines = $stdoutLines; stderrLines = $stderrLines; capturedBytes = $capturedBytes; targetSeen = $targetSeen
      beforeReadable = ($null -ne $before); afterReadable = ($null -ne $after); initTailSeen = $tail; continueSent = $continueSent; hostResumeSeen = $hostResumeSeen
      promptLines = $promptLines; markerMentionLines = $markerMentionLines; numericTargetLines = $numericTargetLines; gflagValueLines = $gflagValueLines; nullCharacters = $nullCharacters; reportedWin32Errors = @($reportedWin32Errors); gflagHints = $gflagHints }
  }
  Send-Protocol @{ state = 'failed'; reason = $firstFailureStage; metadata = $metadata }
  # The Rust guard consumes finished.json before Cargo returns. Retain it until
  # the owning Node controller closes stdin/acknowledges cleanup.
  try {
    if ($null -ne $inputTask -and $inputTask.Wait([TimeSpan]::FromSeconds(120))) {
      if ($null -ne $inputTask.Result -and $inputTask.Result -ne 'CLEAN') {
        $inputTask = $controlReader.ReadLineAsync()
        $inputTask.Wait([TimeSpan]::FromSeconds(120)) | Out-Null
      }
    }
  } catch { }
} finally {
  try { Stop-Cdb } catch { $debuggerClosed = $false }
  if ($null -ne $writer) { $writer.Dispose() }
  if ($null -ne $debugger) { $debugger.Dispose() }
  try { $controlReader.Dispose() } catch { }
  $removed = $false
  if ($ownsRaw) { try { Remove-Item -LiteralPath $RawDirectory -Recurse -Force; $removed = -not (Test-Path -LiteralPath $RawDirectory) } catch { } }
  Send-Protocol @{ state = 'cleaned'; ownedStopSucceeded = ($debuggerClosed -and $symbolState.closed); rawRemoved = $removed; rawReady = $false }
}
