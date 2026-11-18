$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-child-block-event.ps1')
$stamp = '2026-01-01T10:00:00.0123456Z'
$ticks = ([datetimeoffset]::Parse($stamp)).UtcDateTime.ToFileTimeUtc()
$xml = @'
<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><System><Provider Name="Microsoft-Windows-Security-Mitigations"/><Channel>Microsoft-Windows-Security-Mitigations/KernelMode</Channel><EventID>4</EventID></System><EventData>
<Data Name="ProcessPath">\Device\HarddiskVolume3\private path\node.exe</Data><Data Name="ProcessCommandLine">"C:\private path\node.exe" "-e" "trusted parent script"</Data>
<Data Name="CallingProcessId">1234</Data><Data Name="CallingThreadId">4321</Data><Data Name="CallingProcessCreateTime">2026-01-01T10:00:00.0123456Z</Data>
<Data Name="ChildImagePathName">C:\private path\node.exe</Data><Data Name="ChildCommandLine">"C:\private path\node.exe" -e "require('node:fs').writeFileSync('child-marker', 'bad')"</Data>
</EventData></Event>
'@
$cases = @(
    @($xml, 'matched'),
    @($xml.Replace('>4<','>3<'), 'no-match'),
    @($xml.Replace('KernelMode','UserMode'), 'no-match'),
    @($xml.Replace('Name="Microsoft-Windows-Security-Mitigations"','Name="foreign"'), 'no-match'),
    @($xml.Replace('>1234<','>1235<'), 'no-match'),
    @($xml.Replace('>4321<','>4322<'), 'no-match'),
    @($xml.Replace('0123456Z','0123457Z'), 'no-match'),
    @($xml.Replace('0123456Z','012345600Z'), 'matched'),
    @($xml.Replace('0123456Z','012345601Z'), 'no-match'),
    @($xml.Replace('CallingProcessId','ForeignProcessId'), 'no-match'),
    @($xml.Replace('>C:\private path\node.exe<','>D:\other\node.exe<').Replace('C:\private path\node.exe" -e', 'D:\other\node.exe" -e'), 'request-unverified'),
    @($xml.Replace('>C:\private path\node.exe<','>\Device\HarddiskVolume3\private path\node.exe<'), 'matched'),
    @($xml.Replace('child-marker','other-marker'), 'request-unverified'),
    @($xml.Replace('C:\private path\node.exe" -e', 'D:\other\node.exe" -e'), 'request-unverified')
)
foreach ($case in $cases) {
    $result = Get-DocumentChildEventMatch ([xml]$case[0]) 1234 4321 $ticks
    if ($result -cne $case[1]) { throw "Unexpected closed verdict: $result, expected $($case[1])" }
}
# Test lookup and aggregation without reading the local system's event log.
$script:enabled = $true
$script:eventCopies = 1
function Get-WinEvent($ListLog, $FilterHashtable, $MaxEvents) {
    if ($ListLog) { return @{IsEnabled=$script:enabled} }
    if ($FilterHashtable.ContainsKey('EndTime') -or $FilterHashtable.StartTime -cne $stamp) { throw 'lossy query time bound' }
    if ($FilterHashtable.Id -ne 4 -or $FilterHashtable.CallingProcessId -ne '1234' -or $MaxEvents -ne 33) { throw 'wrong query' }
    for ($i = 0; $i -lt $script:eventCopies; $i++) {
        $record = [pscustomobject]@{Raw=$xml}
        $record | Add-Member -MemberType ScriptMethod -Name ToXml -Value { $this.Raw }
        $record | Add-Member -MemberType ScriptMethod -Name Dispose -Value {}
        $record
    }
}
$env:OFOX_DOCUMENT_CHILD_BLOCK_EVENT = '0'
$env:GITHUB_ACTIONS = 'true'
$env:GITHUB_WORKFLOW = 'Document Engine Development Checks'
if ((Read-DocumentChildBlockEvent 1234 4321 $ticks) -cne 'query-unavailable') { throw 'missing opt-in' }
$env:OFOX_DOCUMENT_CHILD_BLOCK_EVENT = '1'
foreach ($case in @(@(1,'matched'),@(0,'no-match'),@(2,'ambiguous'),@(33,'record-limit'))) {
    $script:eventCopies = $case[0]
    if ((Read-DocumentChildBlockEvent 1234 4321 $ticks) -cne $case[1]) { throw 'wrong aggregate verdict' }
}
$script:enabled = $false
if ((Read-DocumentChildBlockEvent 1234 4321 $ticks) -cne 'channel-disabled') { throw 'disabled channel' }
'passed'
