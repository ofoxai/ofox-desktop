# Read-only, CI-only witness for one failed fixed child-creation probe.
param([uint32]$CallerId, [uint32]$CallerTid, [long]$CreationTime, [long]$ExitTime)

function Remove-DocumentPathPrefix([string]$Path) {
    foreach ($prefix in @('\\?\','\??\')) {
        if ($Path.StartsWith($prefix, [StringComparison]::Ordinal)) { return $Path.Substring($prefix.Length) }
    }
    return $Path
}

function ConvertTo-DocumentEventFileTime([string]$Timestamp) {
    # Event XML may zero-pad the 100 ns FILETIME value to nine fractional digits.
    $Timestamp = $Timestamp -creplace '(\.\d{7})0{1,2}Z$', '$1Z'
    $stamp = [datetimeoffset]::MinValue
    if ([datetimeoffset]::TryParseExact($Timestamp, "yyyy-MM-dd'T'HH:mm:ss.FFFFFFF'Z'",
            [cultureinfo]::InvariantCulture, [Globalization.DateTimeStyles]::AssumeUniversal, [ref]$stamp) -and
        $stamp.UtcDateTime -ge [datetime]::FromFileTimeUtc(0)) { return $stamp.UtcDateTime.ToFileTimeUtc() }
    return $null
}

function Get-DocumentChildEventMatch([xml]$Record, [uint32]$CallerId, [uint32]$CallerTid, [long]$CreationTime, [long]$ExitTime) {
    $ns = [System.Xml.XmlNamespaceManager]::new($Record.NameTable)
    $ns.AddNamespace('e', 'http://schemas.microsoft.com/win/2004/08/events/event')
    $system = $Record.SelectSingleNode('/e:Event/e:System', $ns)
    if ($null -eq $system -or $system.SelectSingleNode('e:Provider', $ns).GetAttribute('Name') -cne 'Microsoft-Windows-Security-Mitigations' -or
        $system.SelectSingleNode('e:Channel', $ns).InnerText -cne 'Microsoft-Windows-Security-Mitigations/KernelMode' -or
        $system.SelectSingleNode('e:EventID', $ns).InnerText -cne '4') { return 'identity-unverified' }

    $timestampNode = $system.SelectSingleNode('e:TimeCreated/@SystemTime', $ns)
    $eventTime = ConvertTo-DocumentEventFileTime $timestampNode.Value
    if ($null -ne $eventTime -and ($eventTime -lt $CreationTime -or $eventTime -gt $ExitTime)) { return 'outside-window' }

    $pidNodes = $Record.SelectNodes('/e:Event/e:EventData/e:Data[@Name="CallingProcessId"]', $ns)
    $recordPid = [uint32]0
    $knownPid = $pidNodes.Count -eq 1 -and [uint32]::TryParse($pidNodes[0].InnerText, [ref]$recordPid) -and $recordPid -gt 0
    if ($null -eq $eventTime) {
        if ($knownPid -and $recordPid -ne $CallerId) { return 'foreign-window-unverified' }
        return 'window-unverified'
    }
    if (-not $knownPid) { return 'identity-unverified' }
    if ($recordPid -ne $CallerId) { return 'caller-absent' }

    $fields = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
    foreach ($node in $Record.SelectNodes('/e:Event/e:EventData/e:Data', $ns)) {
        $name = $node.GetAttribute('Name')
        if ($name -cnotin @('ProcessPath','ProcessCommandLine','CallingThreadId','CallingProcessCreateTime','ChildImagePathName','ChildCommandLine')) { continue }
        if ($fields.ContainsKey($name)) { return 'identity-unverified' }
        $fields.Add($name, $node.InnerText)
    }
    if ((ConvertTo-DocumentEventFileTime $fields['CallingProcessCreateTime']) -ne $CreationTime) { return 'lifetime-unverified' }
    if ($fields['CallingThreadId'] -cne $CallerTid.ToString()) { return 'thread-unverified' }

    $image = $fields['ChildImagePathName']
    $command = $fields['ChildCommandLine']
    $suffix = ' -e "require(''node:fs'').writeFileSync(''child-marker'', ''bad'')"'
    if (-not $image -or -not $image.EndsWith('\node.exe', [StringComparison]::OrdinalIgnoreCase) -or
        -not $command -or -not $command.EndsWith($suffix, [StringComparison]::Ordinal)) { return 'request-unverified' }
    $argv0 = $command.Substring(0, $command.Length - $suffix.Length)
    if ($argv0 -notmatch '^(?:"[^"\r\n]+\\node\.exe"|[^"\s]+\\node\.exe)$') { return 'request-unverified' }
    # The fixed probe asks for process.execPath. The Host quotes the caller's
    # executable at argv[0]. Windows can report the image in NT device form.
    if ($fields['ProcessCommandLine'] -notmatch '^"([^"\r\n]+\\node\.exe)" ') { return 'request-unverified' }
    $callerExe = Remove-DocumentPathPrefix $Matches[1]
    $callerImage = Remove-DocumentPathPrefix $fields['ProcessPath']
    $exe = Remove-DocumentPathPrefix ($argv0.Trim('"'))
    $image = Remove-DocumentPathPrefix $image
    if (-not [string]::Equals($exe, $callerExe, [StringComparison]::OrdinalIgnoreCase)) { return 'request-unverified' }
    if (-not [string]::Equals($image, $exe, [StringComparison]::OrdinalIgnoreCase) -and
        -not [string]::Equals($image, $callerImage, [StringComparison]::OrdinalIgnoreCase)) { return 'request-unverified' }
    return 'matched'
}

function New-DocumentChildEventReader([string]$Channel, [string]$Filter) {
    $query = [System.Diagnostics.Eventing.Reader.EventLogQuery]::new($Channel, [System.Diagnostics.Eventing.Reader.PathType]::LogName, $Filter)
    $query.TolerateQueryErrors = $false
    return [System.Diagnostics.Eventing.Reader.EventLogReader]::new($query)
}

function Read-DocumentChildBlockEvent([uint32]$CallerId, [uint32]$CallerTid, [long]$CreationTime, [long]$ExitTime) {
    if ($env:GITHUB_ACTIONS -cne 'true' -or $env:GITHUB_WORKFLOW -cne 'Document Engine Development Checks' -or
        $env:OFOX_DOCUMENT_CHILD_BLOCK_EVENT -cne '1') { return 'query-unavailable' }
    if ($CreationTime -le 0 -or $ExitTime -le $CreationTime -or $ExitTime - $CreationTime -gt [timespan]::TicksPerMinute) {
        return 'identity-unavailable'
    }
    $ErrorActionPreference = 'Stop'
    $channel = 'Microsoft-Windows-Security-Mitigations/KernelMode'
    $config = $reader = $null
    try {
        $config = Get-WinEvent -ListLog $channel
        if ($null -eq $config) { return 'query-unavailable' }
        if (-not $config.IsEnabled) { return 'channel-disabled' }
        # Query the enclosing milliseconds, then check the exact event timestamp.
        $start = [datetime]::FromFileTimeUtc($CreationTime).ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", [cultureinfo]::InvariantCulture)
        $upper = [datetime]::FromFileTimeUtc($ExitTime)
        $remainder = $ExitTime % [timespan]::TicksPerMillisecond
        if ($remainder) { $upper = $upper.AddTicks([timespan]::TicksPerMillisecond - $remainder) }
        $end = $upper.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", [cultureinfo]::InvariantCulture)
        $filter = "*[System[Provider[@Name='Microsoft-Windows-Security-Mitigations'] and EventID=4 and TimeCreated[@SystemTime>='$start' and @SystemTime<='$end']]]"
        $reader = New-DocumentChildEventReader $channel $filter
        $reader.BatchSize = 1
        $observed = [Collections.Generic.List[string]]::new()
        for ($count = 0; ; $count++) {
            $record = $reader.ReadEvent()
            if ($null -eq $record) { break }
            try {
                if ($count -eq 32) { return 'record-limit' }
                $observed.Add((Get-DocumentChildEventMatch ([xml]$record.ToXml()) $CallerId $CallerTid $CreationTime $ExitTime))
            } finally { $record.Dispose() }
        }
        if ($observed -ccontains 'matched') {
            # Only records positively excluded by caller or event time cannot
            # affect uniqueness. Missing fields leave a possible second match.
            $possible = @($observed | Where-Object { $_ -cnotin @('outside-window','caller-absent','foreign-window-unverified') })
            if ($possible.Count -eq 1) { return 'matched' }
            return 'ambiguous'
        }
        foreach ($stage in @('request-unverified','thread-unverified','lifetime-unverified','identity-unverified','window-unverified','caller-absent')) {
            if ($observed -ccontains $stage) { return $stage }
        }
        if ($observed -ccontains 'foreign-window-unverified') { return 'window-unverified' }
        return 'no-event-in-window'
    } catch {
        return 'query-unavailable'
    } finally {
        if ($null -ne $reader) { $reader.Dispose() }
        if ($null -ne $config) { $config.Dispose() }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    Read-DocumentChildBlockEvent $CallerId $CallerTid $CreationTime $ExitTime
}
