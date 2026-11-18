$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-child-block-event.ps1')
$stamp = '2026-01-01T10:00:00.0123456Z'
$ticks = ([datetimeoffset]::Parse($stamp)).UtcDateTime.ToFileTimeUtc()
$exitStamp = '2026-01-01T10:00:01.0123456Z'
$exitTicks = ([datetimeoffset]::Parse($exitStamp)).UtcDateTime.ToFileTimeUtc()
$xml = @'
<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><System><Provider Name="Microsoft-Windows-Security-Mitigations"/><TimeCreated SystemTime="2026-01-01T10:00:00.0234567Z"/><Channel>Microsoft-Windows-Security-Mitigations/KernelMode</Channel><EventID>4</EventID></System><EventData>
<Data Name="ProcessPath">\Device\HarddiskVolume3\private path\node.exe</Data><Data Name="ProcessCommandLine">"C:\private path\node.exe" "-e" "trusted parent script"</Data>
<Data Name="CallingProcessId">1234</Data><Data Name="CallingThreadId">4321</Data><Data Name="CallingProcessCreateTime">2026-01-01T10:00:00.0123456Z</Data>
<Data Name="ChildImagePathName">C:\private path\node.exe</Data><Data Name="ChildCommandLine">"C:\private path\node.exe" -e "require('node:fs').writeFileSync('child-marker', 'bad')"</Data>
</EventData></Event>
'@
$cases = @(
    @($xml, 'matched'),
    @($xml.Replace('00.0234567Z', '00.0123456Z'), 'matched'),
    @($xml.Replace('00.0234567Z', '01.0123456Z'), 'matched'),
    @($xml.Replace('00.0234567Z', '01.0123457Z'), 'outside-window'),
    @($xml.Replace('00.0234567Z', '00.0123455Z'), 'outside-window'),
    @($xml.Replace('SystemTime=', 'OtherTime='), 'window-unverified'),
    @($xml.Replace('>4<','>3<'), 'identity-unverified'),
    @($xml.Replace('KernelMode','UserMode'), 'identity-unverified'),
    @($xml.Replace('Name="Microsoft-Windows-Security-Mitigations"','Name="foreign"'), 'identity-unverified'),
    @($xml.Replace('>1234<','>1235<'), 'caller-absent'),
    @($xml.Replace('>4321<','>4322<'), 'thread-unverified'),
    @($xml.Replace('0123456Z','0123457Z'), 'lifetime-unverified'),
    @($xml.Replace('0123456Z','012345600Z'), 'matched'),
    @($xml.Replace('0123456Z','012345601Z'), 'lifetime-unverified'),
    @($xml.Replace('CallingProcessId','ForeignProcessId'), 'identity-unverified'),
    @($xml.Replace('>C:\private path\node.exe<','>D:\other\node.exe<').Replace('C:\private path\node.exe" -e', 'D:\other\node.exe" -e'), 'request-unverified'),
    @($xml.Replace('>C:\private path\node.exe<','>\Device\HarddiskVolume3\private path\node.exe<'), 'matched'),
    @($xml.Replace('child-marker','other-marker'), 'request-unverified'),
    @($xml.Replace('C:\private path\node.exe" -e', 'D:\other\node.exe" -e'), 'request-unverified')
)
foreach ($case in $cases) {
    $result = Get-DocumentChildEventMatch ([xml]$case[0]) 1234 4321 $ticks $exitTicks
    if ($result -cne $case[1]) { throw "Unexpected closed verdict: $result, expected $($case[1])" }
}
# Test lookup and aggregation without reading the local system's event log.
$script:enabled = $true
$script:eventCopies = 1
$script:recordXml = $xml
function Get-WinEvent($ListLog) {
    if ($ListLog -cne 'Microsoft-Windows-Security-Mitigations/KernelMode') { throw 'wrong channel' }
    $config = [pscustomobject]@{IsEnabled=$script:enabled}
    $config | Add-Member -MemberType ScriptMethod -Name Dispose -Value {}
    return $config
}
function New-DocumentChildEventReader($Channel, $Filter) {
    $expected = "*[System[Provider[@Name='Microsoft-Windows-Security-Mitigations'] and EventID=4 and TimeCreated[@SystemTime>='2026-01-01T10:00:00.012Z' and @SystemTime<='2026-01-01T10:00:01.013Z']]]"
    if ($Channel -cne 'Microsoft-Windows-Security-Mitigations/KernelMode' -or $Filter -cne $expected) { throw 'wrong query' }
    if ($script:queryError -eq 'open') { throw 'open error' }
    $reader = [pscustomobject]@{Remaining=$script:eventCopies; BatchSize=0}
    $reader | Add-Member -MemberType ScriptMethod -Name ReadEvent -Value {
        if ($this.BatchSize -ne 1 -or $script:queryError -eq 'read') { throw 'read error' }
        if ($this.Remaining -eq 0) { return $null }
        $this.Remaining--
        $sourceXml = if ($script:mixedXml) { $script:mixedXml[$this.Remaining] } else { $script:recordXml }
        $record = [pscustomobject]@{Raw=$sourceXml}
        $record | Add-Member -MemberType ScriptMethod -Name ToXml -Value { $this.Raw }
        $record | Add-Member -MemberType ScriptMethod -Name Dispose -Value {}
        return $record
    }
    $reader | Add-Member -MemberType ScriptMethod -Name Dispose -Value {}
    return $reader
}
$env:OFOX_DOCUMENT_CHILD_BLOCK_EVENT = '0'
$env:GITHUB_ACTIONS = 'true'
$env:GITHUB_WORKFLOW = 'Document Engine Development Checks'
if ((Read-DocumentChildBlockEvent 1234 4321 $ticks $exitTicks) -cne 'query-unavailable') { throw 'missing opt-in' }
$env:OFOX_DOCUMENT_CHILD_BLOCK_EVENT = '1'
foreach ($case in @(@(1,'matched'),@(0,'no-event-in-window'),@(2,'ambiguous'),@(33,'record-limit'))) {
    $script:eventCopies = $case[0]
    $result = Read-DocumentChildBlockEvent 1234 4321 $ticks $exitTicks
    if ($result -cne $case[1]) { throw "Aggregate: expected $($case[1]), got $result" }
}
$script:eventCopies = 1
foreach ($case in @(
    @($xml.Replace('>1234<','>4567<'), 'caller-absent'),
    @($xml.Replace('CallingProcessId','OtherId'), 'identity-unverified'),
    @($xml.Replace('00.0234567Z', '01.0123457Z'), 'no-event-in-window'),
    @($xml.Replace('SystemTime=', 'OtherTime='), 'window-unverified'),
    @($xml.Replace('0123456Z','0123457Z'), 'lifetime-unverified'),
    @($xml.Replace('>4321<','>4322<'), 'thread-unverified'),
    @($xml.Replace('child-marker','different'), 'request-unverified')
)) {
    $script:recordXml = $case[0]
    if ((Read-DocumentChildBlockEvent 1234 4321 $ticks $exitTicks) -cne $case[1]) { throw "lost diagnostic stage $($case[1])" }
}
$script:eventCopies = 2
foreach ($case in @(
    @($xml.Replace('>1234<', '>9876<'), 'matched'),
    @($xml.Replace('>1234<', '>9876<').Replace('SystemTime=', 'UnknownTime='), 'matched'),
    @($xml.Replace('SystemTime=', 'UnknownTime='), 'ambiguous'),
    @($xml.Replace('CallingProcessId','MissingId'), 'ambiguous')
)) {
    $script:mixedXml = @($xml, $case[0])
    if ((Read-DocumentChildBlockEvent 1234 4321 $ticks $exitTicks) -cne $case[1]) { throw 'unresolved record masked uniqueness' }
}
$script:mixedXml = $null
foreach ($script:queryError in @('open','read')) {
    if ((Read-DocumentChildBlockEvent 1234 4321 $ticks $exitTicks) -cne 'query-unavailable') { throw 'query failure misclassified' }
}
$script:queryError = $null
$script:enabled = $false
if ((Read-DocumentChildBlockEvent 1234 4321 $ticks $exitTicks) -cne 'channel-disabled') { throw 'disabled channel' }
'passed'
