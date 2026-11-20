param(
  [Parameter(Mandatory = $true)][string]$RawDirectory,
  [ValidateRange(30, 900)][int]$DeadlineSeconds = 900
)

# Development-only controller. Stdout is a closed JSON protocol; native tool
# output and all ETL/XML/NDJSON stay in this private RUNNER_TEMP directory.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
. (Join-Path $PSScriptRoot 'document-engine-stdio.ps1')
$controlReader = New-DocumentControlReader
$session = $null
$rawReady = $false
$failure = 'controller_failure'
$nativeCode = $null
$metadata = $null
$ownsRaw = $false
$compilerCodes = @()

function Send-Protocol($Value) {
  [Console]::Out.WriteLine(($Value | ConvertTo-Json -Depth 12 -Compress))
  [Console]::Out.Flush()
}

function Read-Protocol([int]$Seconds) {
  $read = $controlReader.ReadLineAsync()
  if (-not $read.Wait([TimeSpan]::FromSeconds($Seconds))) { throw 'protocol_deadline' }
  return $read.Result
}

function Find-Tool([string]$Name, [string[]]$Candidates) {
  $found = $null
  foreach ($candidate in $Candidates) {
    if (Test-Path -LiteralPath $candidate -PathType Leaf) { $found = $candidate; break }
  }
  if ($null -eq $found) {
    $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $command) { $found = $command.Source }
  }
  if ($null -eq $found) { return @{ Path = $null; Safe = @{ available = $false } } }
  $item = Get-Item -LiteralPath $found
  $version = $item.VersionInfo
  $signature = Get-AuthenticodeSignature -LiteralPath $found
  return @{
    Path = $found
    Safe = @{
      available = $true
      version = "$($version.FileMajorPart).$($version.FileMinorPart).$($version.FileBuildPart).$($version.FilePrivatePart)"
      sha256 = (Get-FileHash -LiteralPath $found -Algorithm SHA256).Hash.ToLowerInvariant()
      signatureValid = ($signature.Status -eq 'Valid')
      microsoftSigned = ($signature.Status -eq 'Valid' -and $signature.SignerCertificate.Subject -match '(?:^|,\s*)O=Microsoft Corporation(?:,|$)')
    }
  }
}

function Run-BoundedTool([string]$Executable, [string[]]$Arguments, [string]$Label, [string]$Product, [long]$MaximumBytes) {
  $start = [Diagnostics.ProcessStartInfo]::new()
  $start.FileName = $Executable
  $start.WorkingDirectory = $RawDirectory
  $start.UseShellExecute = $false
  $start.CreateNoWindow = $true
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
  $process = [Diagnostics.Process]::new()
  $process.StartInfo = $start
  if (-not $process.Start()) { throw 'tool_start_failed' }
  # Drain both streams concurrently, retaining no messages in the report.
  $stdout = [IO.File]::Create((Join-Path $RawDirectory "$Label.stdout"))
  $stderr = [IO.File]::Create((Join-Path $RawDirectory "$Label.stderr"))
  try {
    $copyOut = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
    $copyErr = $process.StandardError.BaseStream.CopyToAsync($stderr)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (-not $process.WaitForExit(100)) {
      $tooLarge = $stdout.Length -gt 1048576 -or $stderr.Length -gt 1048576
      if (Test-Path -LiteralPath $Product) { $tooLarge = $tooLarge -or (Get-Item -LiteralPath $Product).Length -gt $MaximumBytes }
      if ($tooLarge -or $watch.Elapsed.TotalSeconds -gt 120) {
        $process.Kill($true)
        $process.WaitForExit()
        throw 'tool_bound_exceeded'
      }
    }
    if (-not [Threading.Tasks.Task]::WaitAll(@($copyOut, $copyErr), 5000)) { throw 'tool_drain_failed' }
    if ($stdout.Length -gt 1048576 -or $stderr.Length -gt 1048576) { throw 'tool_bound_exceeded' }
    if ($process.ExitCode -ne 0) { throw 'tool_failed' }
    if (-not (Test-Path -LiteralPath $Product -PathType Leaf) -or (Get-Item -LiteralPath $Product).Length -gt $MaximumBytes) {
      throw 'tool_product_invalid'
    }
  } finally {
    if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
    $stdout.Dispose()
    $stderr.Dispose()
    $process.Dispose()
  }
}

try {
  if (-not $IsWindows -or -not [Environment]::Is64BitProcess) { throw 'native_windows_x64_required' }
  $runner = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd([IO.Path]::DirectorySeparatorChar)
  $runnerPrefix = $runner + [IO.Path]::DirectorySeparatorChar
  $RawDirectory = [IO.Path]::GetFullPath($RawDirectory)
  if (-not $RawDirectory.StartsWith($runnerPrefix, [StringComparison]::OrdinalIgnoreCase) -or
      [IO.Path]::GetFileName($RawDirectory) -notmatch '^ofox-document-etw-[a-f0-9-]{36}$') { throw 'private_directory_invalid' }
  $directory = Get-Item -LiteralPath $RawDirectory
  if (($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
      [IO.Directory]::GetFileSystemEntries($RawDirectory).Length -ne 0) { throw 'private_directory_invalid' }
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  try { $sid = $identity.User.Value } finally { $identity.Dispose() }
  $acl = [Security.AccessControl.DirectorySecurity]::new()
  $acl.SetSecurityDescriptorSddlForm("O:${sid}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;${sid})")
  Set-Acl -LiteralPath $RawDirectory -AclObject $acl
  $ownsRaw = $true

  $failure = 'tool_inventory_failed'
  $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
  $xperf = Find-Tool 'xperf.exe' @((Join-Path $kits 'Windows Performance Toolkit\xperf.exe'))
  $wpr = Find-Tool 'wpr.exe' @((Join-Path $env:SystemRoot 'System32\wpr.exe'), (Join-Path $kits 'Windows Performance Toolkit\wpr.exe'))
  $cdb = Find-Tool 'cdb.exe' @((Join-Path $kits 'Debuggers\x64\cdb.exe'))
  $gflags = Find-Tool 'gflags.exe' @((Join-Path $kits 'Debuggers\x64\gflags.exe'))
  $tracerpt = Find-Tool 'tracerpt.exe' @((Join-Path $env:SystemRoot 'System32\tracerpt.exe'))
  $metadata = @{
    tools = @{ xperf = $xperf.Safe; wpr = $wpr.Safe; cdb = $cdb.Safe; gflags = $gflags.Safe; tracerpt = $tracerpt.Safe }
    userInteractive = [Environment]::UserInteractive
    sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
    flags = @('THREAD', 'LOADER', 'FILE_IO', 'FILE_IO_INIT', 'REGISTRY', 'NO_SYSCONFIG')
    maximumFileMiB = 128
    deadlineSeconds = $DeadlineSeconds
  }
  $failure = 'required_tool_unavailable'
  if (-not $tracerpt.Safe.available -or -not $tracerpt.Safe.microsoftSigned) { throw 'required_tool_unavailable' }

  $failure = 'native_controller_compile_failed'
  Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Xml;

public sealed class DocumentEtwFailure : Exception {
  public string Stage; public uint Code;
  public DocumentEtwFailure(string stage, uint code) { Stage = stage; Code = code; }
}
public sealed class DocumentEtwSession : IDisposable {
  [StructLayout(LayoutKind.Sequential)] struct Wnode {
    public uint BufferSize, ProviderId; public ulong HistoricalContext; public long TimeStamp;
    public Guid Guid; public uint ClientContext, Flags;
  }
  [StructLayout(LayoutKind.Sequential)] struct Properties {
    public Wnode Wnode; public uint BufferSize, MinimumBuffers, MaximumBuffers, MaximumFileSize;
    public uint LogFileMode, FlushTimer, EnableFlags; public int AgeLimit;
    public uint NumberOfBuffers, FreeBuffers, EventsLost, BuffersWritten, LogBuffersLost, RealTimeBuffersLost;
    public IntPtr LoggerThreadId; public uint LogFileNameOffset, LoggerNameOffset;
  }
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode)] static extern uint StartTraceW(out ulong handle, string name, IntPtr properties);
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode)] static extern uint ControlTraceW(ulong handle, string name, IntPtr properties, uint control);
  readonly Guid guid = Guid.NewGuid(); readonly string name, file; readonly int bytes;
  IntPtr memory; ulong handle; bool active;
  public bool OwnedStopSucceeded; public uint EventsLost, LogBuffersLost, RealTimeBuffersLost, BuffersWritten;
  public DocumentEtwSession(string fileName) {
    file = Path.GetFullPath(fileName); name = "OfoxDocument-" + guid.ToString("N");
    int header = Marshal.SizeOf<Properties>();
    byte[] logger = Encoding.Unicode.GetBytes(name + "\0"), path = Encoding.Unicode.GetBytes(file + "\0");
    bytes = checked(header + logger.Length + path.Length);
    memory = Marshal.AllocHGlobal(bytes);
    Marshal.Copy(new byte[bytes], 0, memory, bytes);
    Properties p = new Properties();
    p.Wnode.BufferSize = (uint)bytes; p.Wnode.Guid = guid; p.Wnode.ClientContext = 1; p.Wnode.Flags = 0x00020000;
    p.BufferSize = 64; p.MinimumBuffers = 64; p.MaximumBuffers = 128; p.MaximumFileSize = 128;
    p.LogFileMode = 0x02000002; p.FlushTimer = 1;
    // No PROCESS events (their payload includes UserSID and CommandLine),
    // CPU, stacks, extended SID data, or system-configuration rundown.
    p.EnableFlags = 0x16020006;
    p.LoggerNameOffset = (uint)header; p.LogFileNameOffset = (uint)(header + logger.Length);
    Marshal.StructureToPtr(p, memory, false);
    Marshal.Copy(logger, 0, IntPtr.Add(memory, header), logger.Length);
    Marshal.Copy(path, 0, IntPtr.Add(memory, (int)p.LogFileNameOffset), path.Length);
    uint result = StartTraceW(out handle, name, memory);
    if (result != 0) { Dispose(); throw new DocumentEtwFailure("start_trace", result); }
    active = true;
  }
  string ReadName(uint offset) {
    if (offset >= bytes || (offset & 1) != 0) throw new DocumentEtwFailure("owner_layout", 0);
    var value = new StringBuilder();
    for (int i = (int)offset; i + 1 < bytes; i += 2) {
      char c = (char)(ushort)Marshal.ReadInt16(memory, i);
      if (c == '\0') return value.ToString();
      value.Append(c);
    }
    throw new DocumentEtwFailure("owner_layout", 0);
  }
  public void Stop() {
    if (!active) return;
    uint code = ControlTraceW(handle, null, memory, 0);
    if (code != 0) throw new DocumentEtwFailure("query_trace", code);
    Properties p = Marshal.PtrToStructure<Properties>(memory);
    if (p.Wnode.Guid != guid || ReadName(p.LoggerNameOffset) != name ||
        !String.Equals(ReadName(p.LogFileNameOffset), file, StringComparison.OrdinalIgnoreCase))
      throw new DocumentEtwFailure("owner_mismatch", 0);
    code = ControlTraceW(handle, null, memory, 1);
    if (code != 0) throw new DocumentEtwFailure("stop_trace", code);
    active = false; OwnedStopSucceeded = true;
    p = Marshal.PtrToStructure<Properties>(memory);
    EventsLost = p.EventsLost; LogBuffersLost = p.LogBuffersLost;
    RealTimeBuffersLost = p.RealTimeBuffersLost; BuffersWritten = p.BuffersWritten;
  }
  public void Dispose() { if (memory != IntPtr.Zero) { Marshal.FreeHGlobal(memory); memory = IntPtr.Zero; } }
}
public static class DocumentEtwXml {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern uint QueryDosDeviceW(string device, StringBuilder target, uint characters);
  public static string[] WindowsAliases(string root, out uint code) {
    root = Path.GetFullPath(root).TrimEnd('\\');
    var aliases = new List<string> { root, @"\??\" + root, @"\\?\" + root };
    code = 87;
    if (root.Length >= 3 && root[1] == ':' && root[2] == '\\') {
      var target = new StringBuilder(32768);
      if (QueryDosDeviceW(root.Substring(0, 2), target, 32768) != 0) {
        string device = target.ToString().Split('\0')[0];
        if (device.StartsWith(@"\Device\", StringComparison.OrdinalIgnoreCase)) {
          aliases.Add(device + root.Substring(2)); code = 0;
        }
      } else { code = unchecked((uint)Marshal.GetLastWin32Error()); }
    }
    return aliases.ToArray();
  }
  static readonly HashSet<string> Names = new HashSet<string>(StringComparer.OrdinalIgnoreCase) {
    "ProcessId", "TThreadId", "IrpPtr", "TTID", "FileObject", "CreateOptions", "FileAttributes", "ShareAccess",
    "OpenPath", "ExtraInfo", "NtStatus", "InitialTime", "Status", "Index", "KeyHandle", "KeyName",
    "ImageBase", "ImageSize", "ImageChecksum", "TimeDateStamp", "DefaultBase", "FileName"
  };
  static uint? Number(string value) { uint n; return UInt32.TryParse(value, out n) ? n : (uint?)null; }
  public static long Normalize(string source, string destination) {
    var settings = new XmlReaderSettings { DtdProcessing = DtdProcessing.Prohibit, XmlResolver = null,
      MaxCharactersInDocument = 1073741824, MaxCharactersFromEntities = 0, IgnoreComments = true };
    long count = 0;
    var deadline = System.Diagnostics.Stopwatch.StartNew();
    using (var reader = XmlReader.Create(source, settings))
    using (var output = new StreamWriter(destination, false, new UTF8Encoding(false))) {
      while (reader.Read()) {
        if (reader.NodeType != XmlNodeType.Element || reader.LocalName != "Event") continue;
        if (deadline.Elapsed.TotalSeconds > 120) throw new DocumentEtwFailure("xml_deadline", 0);
        var doc = new XmlDocument { XmlResolver = null };
        using (var subtree = reader.ReadSubtree()) doc.Load(subtree);
        if (doc.OuterXml.Length > 131072 || ++count > 2000000) throw new DocumentEtwFailure("xml_bound", 0);
        var ns = new XmlNamespaceManager(doc.NameTable);
        ns.AddNamespace("e", "http://schemas.microsoft.com/win/2004/08/events/event");
        var system = doc.SelectSingleNode("/e:Event/e:System", ns);
        string provider = system?.SelectSingleNode("e:Provider", ns)?.Attributes?["Guid"]?.Value;
        Guid parsedGuid; provider = Guid.TryParse(provider, out parsedGuid) ? parsedGuid.ToString("D") : null;
        string timestamp = null;
        DateTime utc;
        string time = system?.SelectSingleNode("e:TimeCreated", ns)?.Attributes?["SystemTime"]?.Value;
        if (time != null && time.EndsWith("Z", StringComparison.OrdinalIgnoreCase) &&
            DateTime.TryParse(time, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out utc)) {
          try { timestamp = utc.ToUniversalTime().ToFileTimeUtc().ToString(CultureInfo.InvariantCulture); } catch (ArgumentOutOfRangeException) { }
        }
        var execution = system?.SelectSingleNode("e:Execution", ns);
        var data = new Dictionary<string,string>(StringComparer.Ordinal);
        foreach (XmlNode item in doc.SelectNodes("/e:Event/e:EventData/e:Data", ns)) {
          string key = item.Attributes?["Name"]?.Value;
          if (key == null || !Names.Contains(key)) continue;
          if (item.InnerText.Length > 4096 || data.Count >= 32 || data.ContainsKey(key)) throw new DocumentEtwFailure("xml_field_bound", 0);
          data.Add(key, item.InnerText);
        }
        var record = new { providerGuid = provider,
          opcode = Number(system?.SelectSingleNode("e:Opcode", ns)?.InnerText),
          version = Number(system?.SelectSingleNode("e:Version", ns)?.InnerText), timestamp100ns = timestamp,
          headerPid = Number(execution?.Attributes?["ProcessID"]?.Value),
          headerTid = Number(execution?.Attributes?["ThreadID"]?.Value), data = data };
        output.WriteLine(System.Text.Json.JsonSerializer.Serialize(record));
        if (output.BaseStream.Position > 536870912) throw new DocumentEtwFailure("ndjson_bound", 0);
      }
    }
    return count;
  }
}
'@

  $failure = 'capture_start_failed'
  [uint32]$aliasStatus = 0
  $aliases = [DocumentEtwXml]::WindowsAliases($env:SystemRoot, [ref]$aliasStatus)
  [IO.File]::WriteAllText((Join-Path $RawDirectory 'roots.json'),
    (@{ windows = @($aliases) } | ConvertTo-Json -Depth 3 -Compress), [Text.UTF8Encoding]::new($false))
  $metadata.windowsAliasCount = $aliases.Length
  $metadata.windowsDeviceAliasAvailable = ($aliasStatus -eq 0)
  $metadata.windowsAliasStatus = $aliasStatus
  $raw = Join-Path $RawDirectory 'capture.etl'
  $session = [DocumentEtwSession]::new($raw)
  Send-Protocol @{ state = 'started'; metadata = $metadata }
  $failure = 'capture_deadline'
  if ((Read-Protocol $DeadlineSeconds) -ne 'STOP') { throw 'protocol_invalid' }
  $failure = 'capture_stop_failed'
  $session.Stop()
  $loss = @{
    ownedStopSucceeded = $session.OwnedStopSucceeded; eventsLost = $session.EventsLost
    logBuffersLost = $session.LogBuffersLost; realTimeBuffersLost = $session.RealTimeBuffersLost
    buffersWritten = $session.BuffersWritten
  }
  $failure = 'trace_decode_failed'
  $xml = Join-Path $RawDirectory 'events.xml'
  Run-BoundedTool $tracerpt.Path @($raw, '-o', $xml, '-of', 'XML', '-y') 'xml' $xml 1073741824
  $count = [DocumentEtwXml]::Normalize($xml, (Join-Path $RawDirectory 'events.ndjson'))
  $rawReady = $true
  Send-Protocol @{ state = 'raw-ready'; metadata = $metadata; lossCounters = $loss; eventCount = $count }
  $failure = 'decode_ack_deadline'
  if ((Read-Protocol 120) -ne 'CLEAN') { throw 'protocol_invalid' }
} catch {
  $exception = $_.Exception
  if ($failure -eq 'native_controller_compile_failed') {
    $compilerCodes = @([regex]::Matches($_.ToString(), '\bCS\d{4}\b') | ForEach-Object { $_.Value } | Select-Object -Unique -First 8)
  }
  while ($null -ne $exception.InnerException) { $exception = $exception.InnerException }
  if ($exception.GetType().FullName -eq 'DocumentEtwFailure') { $nativeCode = $exception.Code; $failure = $exception.Stage }
  Send-Protocol @{ state = 'failed'; reason = $failure; nativeCode = $nativeCode; compilerCodes = $compilerCodes; metadata = $metadata }
} finally {
  $stopped = $false
  if ($null -ne $session) {
    try { $session.Stop(); $stopped = $session.OwnedStopSucceeded } catch { $stopped = $false }
    $session.Dispose()
  }
  $removed = $false
  try {
    if ($ownsRaw) {
      if (Test-Path -LiteralPath $RawDirectory) { Remove-Item -LiteralPath $RawDirectory -Recurse -Force }
      $removed = -not (Test-Path -LiteralPath $RawDirectory)
    }
  } catch { }
  try { $controlReader.Dispose() } catch { }
  Send-Protocol @{ state = 'cleaned'; ownedStopSucceeded = $stopped; rawRemoved = $removed; rawReady = $rawReady }
}
