param([string]$Mode = 'valid')
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-engine-operations.ps1')
$state = New-DocumentOperationSetup 42 43
$definitions = @(Get-DocumentOperationDefinitions)
$lines = @('OFOX_NT_RAW_V1 LAYOUT oa=30 root=8 name=10 us=10 length=0 buffer=8 END',
  'OFOX_NT_RAW_V1 THREADS_BEGIN', '   0 Id: 2a.2c Suspend: 1 Teb: private-address Unfrozen',
  '.  1 Id: 2a.2b Suspend: 1 Teb: private-address Unfrozen', 'OFOX_NT_RAW_V1 THREADS_END',
  'OFOX_NT_RAW_V1 BREAKPOINTS_BEGIN')
for ($index = 0; $index -lt 14; $index++) {
  $name = $definitions[$index].Name
  if ($Mode -eq 'alias') { $name = 'Zw' + $name.Substring(2) }
  $lines += ('{0} e 00007fff`12340000 0001 (0001) 0:001 ntdll!{1} "private command"' -f ($index + 10),$name)
}
$lines += 'OFOX_NT_RAW_V1 BREAKPOINTS_END'
if ($Mode -eq 'layout') { $lines[0] = $lines[0].Replace('oa=30', 'oa=20') }
if ($Mode -eq 'foreign') { $lines[3] = $lines[3].Replace('2a.2b', '2c.2b') }
if ($Mode -eq 'wrongthread') { $lines[6] = $lines[6].Replace('0:001', '0:000') }
if ($Mode -eq 'unbound') { $lines[6] = $lines[6].Replace('0:001', '0:***') }
if ($Mode -eq 'disabled') { $lines[6] = $lines[6].Replace(' e ', ' d ') }
if ($Mode -eq 'unresolved') { $lines[6] = $lines[6].Replace(' e ', ' eu ') }
if ($Mode -eq 'duplicate') { $lines[7] = $lines[6] }
if ($Mode -eq 'symbol') { $lines[6] = $lines[6].Replace('NtOpenKey', 'NtOpenFile') }
if ($Mode -eq 'missing') { $lines = $lines[0..($lines.Count-3)] + $lines[-1] }
if ($Mode -eq 'extra') { $lines = $lines[0..($lines.Count-2)] + @('private unrecognized row', $lines[-1]) }
$failure = $null
try { foreach ($line in $lines) { Read-DocumentOperationSetupLine $state $line }; Assert-DocumentOperationSetup $state } catch { $failure = $_.Exception.Message }
$program = Get-DocumentOperationProgram 43
[Console]::WriteLine((@{ verified = ($null -eq $failure); failure = $failure; threadIndex = $state.threadIndex
  program = $program; definitions = $definitions } | ConvertTo-Json -Compress -Depth 8))
