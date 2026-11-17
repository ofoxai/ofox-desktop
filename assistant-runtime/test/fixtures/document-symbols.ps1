param([Parameter(Mandatory = $true)][string]$Mode)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-engine-symbols.ps1')
if ($Mode -eq 'shape') {
  $text = "SYMCHK: C:\private-account-123\ntdll.dll PASSED`nSYMCHK: FAILED files = 0`nSYMCHK: PASSED + IGNORED files = 1`nprivate-token S-1-5-21-12345`n"
  $utf8 = [Text.Encoding]::UTF8.GetBytes($text)
  $utf16 = [byte[]](@(255,254) + @([Text.Encoding]::Unicode.GetBytes($text)))
  [Console]::Out.WriteLine((@{ plain = (Get-DocumentSymbolOutputShape $utf8 'C:\private-account-123\ntdll.dll')
    wide = (Get-DocumentSymbolOutputShape $utf16 'C:\private-account-123\ntdll.dll') } | ConvertTo-Json -Depth 6 -Compress))
  exit
}
if ($Mode -eq 'success') {
  $tokens = $null; $errors = $null
  $ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot '../../../scripts/document-engine-cdb.ps1'), [ref]$tokens, [ref]$errors)
  if ($errors.Count) { throw 'controller_parse_failed' }
  $definition = $ast.Find({ param($node)
    $node -is [System.Management.Automation.Language.CommandAst] -and $node.GetCommandName() -eq 'Add-Type'
  }, $true)
  . ([ScriptBlock]::Create($definition.Extent.Text))
  $info = [DocumentCdbNative].GetNestedType('FileInfo', [Reflection.BindingFlags]::NonPublic)
  $size = [Runtime.InteropServices.Marshal].GetMethod('SizeOf', [type[]]@([type])).Invoke($null, @($info))
  if ($size -ne 52) { throw 'native_file_info_layout' }
}
$state = @{ started = $false; closed = $true; exitCode = $null; failedFiles = $null; passedOrIgnoredFiles = $null; passedFile = $false }
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = (Get-Process -Id $PID).Path
$start.UseShellExecute = $false; $start.CreateNoWindow = $true
$start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
$command = switch ($Mode) {
  'success' { '[Console]::Out.WriteLine("private-token"); [Console]::Out.WriteLine("SYMCHK: ntdll.dll PASSED"); [Console]::Out.WriteLine("SYMCHK: FAILED files = 0"); [Console]::Out.WriteLine("SYMCHK: PASSED + IGNORED files = 1")' }
  'nonzero' { '[Console]::Out.WriteLine("SYMCHK: ntdll.dll PASSED"); [Console]::Out.WriteLine("SYMCHK: FAILED files = 0"); [Console]::Out.WriteLine("SYMCHK: PASSED + IGNORED files = 1"); exit 7' }
  'empty' { '[Console]::Out.WriteLine("SYMCHK: ntdll.dll PASSED"); [Console]::Out.WriteLine("SYMCHK: FAILED files = 0"); [Console]::Out.WriteLine("SYMCHK: PASSED + IGNORED files = 0")' }
  'duplicate' { '[Console]::Out.WriteLine("SYMCHK: ntdll.dll PASSED"); [Console]::Out.WriteLine("SYMCHK: FAILED files = 0"); [Console]::Out.WriteLine("SYMCHK: FAILED files = 0"); [Console]::Out.WriteLine("SYMCHK: PASSED + IGNORED files = 1")' }
  'ignored' { '[Console]::Out.WriteLine("SYMCHK: FAILED files = 0"); [Console]::Out.WriteLine("SYMCHK: PASSED + IGNORED files = 1")' }
  'stderr' { '[Console]::Error.WriteLine("SYMCHK: FAILED files = 0"); [Console]::Error.WriteLine("SYMCHK: PASSED + IGNORED files = 1")' }
  'bound' { '[Console]::Out.Write(("x" * 4097)); [Console]::Out.Flush(); Start-Sleep -Seconds 30' }
  'stderrbound' { '[Console]::Error.Write(("x" * 4097)); [Console]::Error.Flush(); Start-Sleep -Seconds 30' }
  'totalbound' { 'for ($i=0;$i -lt 100;$i++) { [Console]::Out.WriteLine(("x" * 1024)) }; Start-Sleep -Seconds 30' }
  default { 'Start-Sleep -Seconds 30' }
}
foreach ($arg in @('-NoLogo','-NoProfile','-NonInteractive','-Command',$command)) { $start.ArgumentList.Add($arg) }
$cancel = [Threading.Tasks.TaskCompletionSource[string]]::new()
if ($Mode -eq 'cancel') { $cancel.SetResult('STOP') }
$control = if ($Mode -eq 'livecancel') { [Threading.Tasks.Task]::Delay(500) } else { $cancel.Task }
$failure = $null
try { Invoke-DocumentSymbolCheck $start $control $state $(if ($Mode -eq 'deadline') { 100 } else { 5000 }) }
catch { $failure = $_.Exception.Message }
[Console]::Out.WriteLine((@{ state = $state; failure = $failure
  arguments = @(Get-DocumentSymbolArguments 'private image' 'private cache') } | ConvertTo-Json -Depth 10 -Compress))
