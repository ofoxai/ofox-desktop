param([string]$Mode = 'valid')
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-engine-operations.ps1')
$state = New-DocumentOperationSetup 42 43
$definitions = @(Get-DocumentOperationDefinitions)
# Production form: the controller reads Rust's receipt with ConvertFrom-Json (Int64).
$architecture = '{"apiSucceeded":true,"processMachine":0,"nativeMachine":34404,"compiledLayoutVerified":true}' | ConvertFrom-Json
if ($Mode -eq 'architecture-float') { $architecture = '{"apiSucceeded":true,"processMachine":0,"nativeMachine":34404.0,"compiledLayoutVerified":true}' | ConvertFrom-Json }
if ($Mode -eq 'architecture-string') { $architecture = '{"apiSucceeded":true,"processMachine":"0","nativeMachine":34404,"compiledLayoutVerified":true}' | ConvertFrom-Json }
if ($Mode -eq 'layout') { $architecture.compiledLayoutVerified = $false }
if ($Mode -eq 'api-failed') { $architecture.apiSucceeded = $false }
if ($Mode -eq 'wow64') { $architecture.processMachine = 0x14c }
if ($Mode -eq 'arm64') { $architecture.nativeMachine = 0xaa64 }
if ($Mode -eq 'architecture-type') { $architecture.nativeMachine = $true }
if ($Mode -eq 'architecture-missing') { $architecture = $null }
$lines = @('OFOX_NT_RAW_V1 THREADS_BEGIN', '   0 Id: 2a.2c Suspend: 1 Teb: private-address Unfrozen',
  '.  1 Id: 2a.2b Suspend: 1 Teb: private-address Unfrozen', 'OFOX_NT_RAW_V1 THREADS_END',
  'OFOX_NT_RAW_V1 BREAKPOINTS_BEGIN')
for ($index = 0; $index -lt 14; $index++) {
  $name = $definitions[$index].Name
  if ($Mode -eq 'alias') { $name = 'Zw' + $name.Substring(2) }
  $lines += ('{0} e 00007fff`12340000 0001 (0001) 0:001 ntdll!{1} "private command"' -f ($index + 10),$name)
}
$lines += 'OFOX_NT_RAW_V1 BREAKPOINTS_END'
# Documented current dbgeng layout: DML action text and a thread-qualified column.
if ($Mode -like 'dml*') {
  for ($index = 5; $index -lt 19; $index++) {
    $lines[$index] = '    ' + $lines[$index].Replace(' e 00007fff', ' e Disable Clear  00007fff').Replace(' 0001 (0001) 0:001 ', '     0001 (0001)  0:~001 ')
  }
}
if ($Mode -eq 'dml-disabled') { $lines[5] = $lines[5].Replace(' e Disable Clear ', ' d Enable Clear ') }
if ($Mode -eq 'dml-wrongthread') { $lines[5] = $lines[5].Replace('0:~001', '0:~000') }
if ($Mode -eq 'dml-unbound') { $lines[5] = $lines[5].Replace('0:~001', '0:****') }
if ($Mode -eq 'dml-action') { $lines[5] = $lines[5].Replace('Disable Clear', 'Disable Private') }
# Native run 44034826154: every -cf command is echoed after a prompt.
if ($Mode -eq 'dml-echo') {
  $lines = @($lines[0], '0:000> ~') + $lines[1..2] + @('0:000> .echo OFOX_NT_RAW_V1 THREADS_END', $lines[3], $lines[4], '0:000> bl /L 10-23') +
    $lines[5..18] + @('0:000> .echo OFOX_NT_RAW_V1 BREAKPOINTS_END', $lines[19])
}
if ($Mode -eq 'dml-echo-row') { $lines[6] = '0:000> ' + $lines[6].TrimStart() }
if ($Mode -eq 'foreign') { $lines[2] = $lines[2].Replace('2a.2b', '2c.2b') }
if ($Mode -eq 'wrongthread') { $lines[5] = $lines[5].Replace('0:001', '0:000') }
if ($Mode -eq 'unbound') { $lines[5] = $lines[5].Replace('0:001', '0:***') }
if ($Mode -eq 'disabled') { $lines[5] = $lines[5].Replace(' e ', ' d ') }
if ($Mode -eq 'unresolved') { $lines[5] = $lines[5].Replace(' e ', ' eu ') }
if ($Mode -eq 'duplicate') { $lines[6] = $lines[5] }
if ($Mode -eq 'symbol') { $lines[5] = $lines[5].Replace('NtOpenKey', 'NtOpenFile') }
if ($Mode -eq 'missing') { $lines = $lines[0..($lines.Count-3)] + $lines[-1] }
if ($Mode -eq 'extra') { $lines = $lines[0..($lines.Count-2)] + @('private unrecognized row', $lines[-1]) }
$failure = $null
try { Confirm-DocumentOperationArchitecture $state $architecture; foreach ($line in $lines) { Read-DocumentOperationSetupLine $state $line }; Assert-DocumentOperationSetup $state } catch { $failure = $_.Exception.Message }
$program = Get-DocumentOperationProgram 43
[Console]::WriteLine((@{ verified = ($null -eq $failure); failure = $failure; threadIndex = $state.threadIndex
  program = $program; definitions = $definitions } | ConvertTo-Json -Compress -Depth 8))
