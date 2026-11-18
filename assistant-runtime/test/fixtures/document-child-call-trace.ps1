$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-child-call-cdb.ps1')
$exe = 'C:\synthetic fixture\node.exe'
$armed = 'OFOX_CALL ARMED'
$leave = 'OFOX_CALL EXIT_PAIRED'
$empty = 'OFOX_CALL EXIT_NONE'
$allow = @($armed, 'OFOX_CALL ENTRY', "OFOX_CALL IMAGE $exe", ('OFOX_CALL ARGS "' + $exe + '" -e "require(''node:fs'').writeFileSync(''child-marker'', ''bad'')"'), 'OFOX_CALL FAILED')
$errorValue = 'LastErrorValue: (Win32) 0x16f (367) - synthetic description'
function Replay([string[]]$Lines) {
    $state = New-ChildCallState $exe
    try {
        foreach ($line in $Lines) { Read-ChildCallLine $state $line }
        return Complete-ChildCall $state
    } catch { return 'inconclusive' }
}
function Check($Actual, $Expected) { if ($Actual -cne $Expected) { throw "unexpected fixture result: $Actual (expected $Expected)" } }
Check (Replay @($armed,$empty)) 'not-reached'
Check (Replay ($allow + @($errorValue,'OFOX_CALL RETURN',$leave))) 'child-process-blocked'
Check (Replay ($allow + @('LastErrorValue: (Win32) 0x222','OFOX_CALL RETURN',$leave))) 'other-win32'
Check (Replay @('0:001> .echo OFOX_CALL ARMED',$leave)) 'inconclusive'
Check (Replay ($allow + @('OFOX_CALL RETURN',$leave))) 'inconclusive'
Check (Replay ($allow + @($errorValue,$errorValue,'OFOX_CALL RETURN',$leave))) 'inconclusive'
Check (Replay ($allow + @('LastErrorValue: (Win32) 0x0','OFOX_CALL RETURN',$leave))) 'inconclusive'
Check (Replay ($allow + @($errorValue,'OFOX_CALL RETURN'))) 'inconclusive'
Check (Replay @($armed,'OFOX_CALL ENTRY',$leave)) 'inconclusive'
Check (Replay @($armed,'Unable to insert breakpoint 10 at synthetic address',$leave)) 'inconclusive'
Check (Replay @($armed,'OFOX_CALL INVALID',$leave)) 'inconclusive'
Check (Replay @($armed,$leave,$leave)) 'inconclusive'
$foreign = $allow.Clone(); $foreign[3] += ' other'
Check (Replay ($foreign + @($errorValue,'OFOX_CALL RETURN',$leave))) 'inconclusive'
Check (Replay @($armed,$leave)) 'inconclusive'
Check (Replay ($allow + @($errorValue,'OFOX_CALL RETURN',$empty))) 'inconclusive'
Check (Replay @('0:001> OFOX_CALL ARMED',$empty)) 'not-reached'
Check (Replay ($allow + @('LastErrorValue: (Win32) 0x16f (5) - mismatch','OFOX_CALL RETURN',$leave))) 'inconclusive'
Check (Replay @($armed,'0:001> Unable to insert breakpoint 10 at fixture address',$empty)) 'inconclusive'
Check (Replay ($allow + @("0:001> $errorValue",'OFOX_CALL RETURN',$leave))) 'child-process-blocked'
Check (Replay ($allow[0..3] + @('OFOX_CALL SUCCEEDED','OFOX_CALL RETURN',$leave))) 'succeeded'
Check (Replay ($allow[0..3] + @('OFOX_CALL SUCCEEDED',$errorValue,'OFOX_CALL RETURN',$leave))) 'inconclusive'
if ($IsWindows) {
    # This is the IAT slot in the locked x64 archive in node-downloads.json.
    # Other, shorter names near the end of its PE section must be skipped.
    $node = Join-Path $PSScriptRoot '../../resources/node/node.exe'
    if (Test-Path -LiteralPath $node) { Check (Get-ChildCallImport $node) 30770800 }
}
'passed'
