$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-child-call-cdb.ps1')
$exe = 'C:\synthetic fixture\node.exe'
$typeProof = @('OFOX_CALL FIELD_BEGIN','OFOX_CALL FIELD_READY')
$armed = 'OFOX_CALL ARMED'
$leave = 'OFOX_CALL EXIT_PAIRED'
$empty = 'OFOX_CALL EXIT_NONE'
$allow = @($armed, 'OFOX_CALL ENTRY', "OFOX_CALL IMAGE $exe", ('OFOX_CALL ARGS "' + $exe + '" -e "require(''node:fs'').writeFileSync(''child-marker'', ''bad'')"'), 'OFOX_CALL FAILED')
$errorValue = 'OFOX_CALL RETURN_WIN32 0000016f END'
function Replay([string[]]$Lines, [string[]]$Setup = $typeProof) {
    $state = New-ChildCallState $exe
    try {
        foreach ($line in ($Setup + $Lines)) { Read-ChildCallLine $state $line }
        return Complete-ChildCall $state
    } catch { return 'inconclusive' }
}
function Check($Actual, $Expected) { if ($Actual -cne $Expected) { throw "unexpected fixture result: $Actual (expected $Expected)" } }
$commands = [IO.Directory]::CreateDirectory((Join-Path ([IO.Path]::GetTempPath()) ('ofox-child-call-' + [Guid]::NewGuid().ToString('N')))).FullName
$exitCommand = Join-Path $commands 'exit.cmd'
try {
    Write-ChildCallExitCommand $commands 123
    $bytes = [IO.File]::ReadAllBytes($exitCommand)
    # CDB's $>< flattens a trailing newline into a separator that invalidates q.
    Check $bytes[-1] ([byte][char]'q')
} finally {
    [IO.File]::Delete($exitCommand)
    [IO.Directory]::Delete($commands)
}
$pending = New-ChildCallState $exe
foreach ($line in ($typeProof + $allow)) { Read-ChildCallLine $pending $line }
Check (Get-ChildCallProgress $pending) $null
Read-ChildCallLine $pending $errorValue
Check (Get-ChildCallProgress $pending) 'PAIRED child-process-blocked'
$script:failureStage = 'exit-marker'
$badExit = $false
try { Read-ChildCallLine $pending 'OFOX_CALL EXIT_NONE' } catch { $badExit = $true }
Check $badExit $true
Check $script:failureStage 'return-conflict'
$incomplete = $false
try { Complete-ChildCall $pending } catch { $incomplete = $true }
Check $incomplete $true
Read-ChildCallLine $pending $leave
Check (Get-ChildCallProgress $pending) $null
# The same CDB error can precede or follow the final exit marker. Preserve
# only that ordering and the fixed error family, never the raw error line.
foreach ($phase in 'before-exit-marker','after-exit-marker') {
    foreach ($case in @(
        @{ reason='command-file'; line='0:000> Unable to read command file C:\private-fixture\breakpoint.cmd' },
        @{ reason='syntax'; line='Syntax error in expression: private-fixture' },
        @{ reason='breakpoint'; line='Unable to insert breakpoint 11 at private-fixture' }
    )) {
        $state = New-ChildCallState $exe
        foreach ($line in ($typeProof + $allow + @($errorValue))) { Read-ChildCallLine $state $line }
        if ($phase -eq 'after-exit-marker') { Read-ChildCallLine $state $leave }
        $script:failureStage = 'unset'
        $rejected = $false
        try { Read-ChildCallLine $state $case.line } catch { $rejected = $true }
        Check $rejected $true
        Check $script:failureStage ("return-error/$phase/" + $case.reason)
    }
}
Check (Replay @($armed,$empty)) 'not-reached'
Check (Replay @($armed,$empty) @()) 'inconclusive'
Check (Replay @($armed,$empty) @('OFOX_CALL FIELD_BEGIN')) 'inconclusive'
Check (Replay ($allow + @($errorValue,$leave))) 'child-process-blocked'
Check (Replay ($allow + @('OFOX_CALL RETURN_WIN32 00000222 END',$leave))) 'other-win32'
Check (Replay @('0:001> .echo OFOX_CALL ARMED',$leave)) 'inconclusive'
Check (Replay ($allow + @('OFOX_CALL RETURN',$leave))) 'inconclusive'
Check (Replay ($allow + @($errorValue,'OFOX_CALL RETURN',$leave))) 'inconclusive'
Check (Replay ($allow + @($errorValue,$errorValue,$leave))) 'inconclusive'
Check (Replay ($allow + @('OFOX_CALL RETURN_WIN32 00000000 END',$leave))) 'inconclusive'
Check (Replay ($allow + @($errorValue))) 'inconclusive'
Check (Replay @($armed,'OFOX_CALL ENTRY',$leave)) 'inconclusive'
Check (Replay @($armed,'Unable to insert breakpoint 10 at synthetic address',$leave)) 'inconclusive'
Check (Replay @($armed,'OFOX_CALL INVALID',$leave)) 'inconclusive'
Check (Replay @($armed,$leave,$leave)) 'inconclusive'
$foreign = $allow.Clone(); $foreign[3] += ' other'
Check (Replay ($foreign + @($errorValue,$leave))) 'inconclusive'
Check (Replay @($armed,$leave)) 'inconclusive'
Check (Replay ($allow + @($errorValue,$empty))) 'inconclusive'
Check (Replay @('0:001> OFOX_CALL ARMED',$empty)) 'not-reached'
foreach ($invalid in @(
    'LastErrorValue: (Win32) 0x16f (367) - synthetic description',
    'OFOX_CALL WIN32 0000016f END', 'OFOX_CALL RETURN_WIN32 10000016f END', 'OFOX_CALL RETURN_WIN32 16f END',
    'OFOX_CALL RETURN_WIN32 0000016f END extra', 'OFOX_CALL RETURN_WIN32 ???????? END'
)) {
    Check (Replay ($allow + @($invalid,$leave))) 'inconclusive'
}
Check (Replay ($allow + @('OFOX_CALL RETURN_WIN32 00000005 END',$leave))) 'access-denied'
Check (Replay @($armed,'0:001> Unable to insert breakpoint 10 at fixture address',$empty)) 'inconclusive'
Check (Replay ($allow + @("0:001> $errorValue",$leave))) 'child-process-blocked'
Check (Replay ($allow[0..3] + @('OFOX_CALL RETURN_SUCCESS',$leave))) 'succeeded'
Check (Replay ($allow[0..3] + @('OFOX_CALL RETURN_SUCCESS',$errorValue,$leave))) 'inconclusive'
# A stopped debugger need not produce a later RETURN or process exit. The
# explicit read failure must be reportable as soon as the fixed command ends.
foreach ($case in @(
    @{ prefix=($typeProof + $allow); row='OFOX_CALL READ_ERROR'; stage='last-error-unreadable' },
    @{ prefix=($typeProof + $allow); row='OFOX_CALL RETURN_WIN32 00000000 END'; stage='last-error-zero' },
    @{ prefix=@('OFOX_CALL FIELD_BEGIN'); row='OFOX_CALL FIELD_UNAVAILABLE'; stage='symbols' }
)) {
    $state = New-ChildCallState $exe
    foreach ($line in $case.prefix) { Read-ChildCallLine $state $line }
    $script:failureStage = 'unset'
    $rejected = $false
    try { Read-ChildCallLine $state $case.row } catch { $rejected = $true }
    Check $rejected $true
    Check $script:failureStage $case.stage
}
# CDB emits its diagnostic text before .catch reaches the fallback marker.
foreach ($case in @(
    @{ prefix=($typeProof + $allow); fallback='OFOX_CALL READ_ERROR'; stage='last-error-unreadable' },
    @{ prefix=@('OFOX_CALL FIELD_BEGIN'); fallback='OFOX_CALL FIELD_UNAVAILABLE'; stage='symbols' }
)) {
    $state = New-ChildCallState $exe
    foreach ($line in $case.prefix) { Read-ChildCallLine $state $line }
    $script:failureStage = 'unset'
    try {
        Read-ChildCallLine $state 'Syntax error in expression'
        Read-ChildCallLine $state $case.fallback
    } catch {}
    Check $script:failureStage $case.stage
}
if ($IsWindows) {
    # This is the IAT slot in the locked x64 archive in node-downloads.json.
    # Other, shorter names near the end of its PE section must be skipped.
    $node = Join-Path $PSScriptRoot '../../resources/node/node.exe'
    if (Test-Path -LiteralPath $node) { Check (Get-ChildCallImport $node) 30770800 }
}
'passed'
