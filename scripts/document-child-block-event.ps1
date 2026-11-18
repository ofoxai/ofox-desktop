# Read-only, CI-only witness for one failed fixed child-creation probe.
param([uint32]$CallerId, [uint32]$CallerTid, [long]$CreationTime)

function Remove-DocumentPathPrefix([string]$Path) {
    foreach ($prefix in @('\\?\','\??\')) {
        if ($Path.StartsWith($prefix, [StringComparison]::Ordinal)) { return $Path.Substring($prefix.Length) }
    }
    return $Path
}

function Get-DocumentChildEventMatch([xml]$Record, [uint32]$CallerId, [uint32]$CallerTid, [long]$CreationTime) {
    $ns = [System.Xml.XmlNamespaceManager]::new($Record.NameTable)
    $ns.AddNamespace('e', 'http://schemas.microsoft.com/win/2004/08/events/event')
    $system = $Record.SelectSingleNode('/e:Event/e:System', $ns)
    if ($null -eq $system -or $system.SelectSingleNode('e:Provider', $ns).GetAttribute('Name') -cne 'Microsoft-Windows-Security-Mitigations' -or
        $system.SelectSingleNode('e:Channel', $ns).InnerText -cne 'Microsoft-Windows-Security-Mitigations/KernelMode' -or
        $system.SelectSingleNode('e:EventID', $ns).InnerText -cne '4') { return 'no-match' }

    $fields = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
    foreach ($node in $Record.SelectNodes('/e:Event/e:EventData/e:Data', $ns)) {
        $name = $node.GetAttribute('Name')
        if ($name -cnotin @('ProcessPath','ProcessCommandLine','CallingProcessId','CallingThreadId','CallingProcessCreateTime','ChildImagePathName','ChildCommandLine')) { continue }
        if ($fields.ContainsKey($name)) { return 'no-match' }
        $fields.Add($name, $node.InnerText)
    }
    if ($fields['CallingProcessId'] -cne $CallerId.ToString() -or $fields['CallingThreadId'] -cne $CallerTid.ToString()) { return 'no-match' }
    $stamp = [datetimeoffset]::MinValue
    # Event XML may zero-pad the 100 ns FILETIME value to nine fractional digits.
    $timestamp = $fields['CallingProcessCreateTime'] -creplace '(\.\d{7})0{1,2}Z$', '$1Z'
    if (-not [datetimeoffset]::TryParseExact($timestamp, "yyyy-MM-dd'T'HH:mm:ss.FFFFFFF'Z'",
            [cultureinfo]::InvariantCulture, [Globalization.DateTimeStyles]::AssumeUniversal, [ref]$stamp) -or
        $stamp.UtcDateTime.ToFileTimeUtc() -ne $CreationTime) { return 'no-match' }

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

function Read-DocumentChildBlockEvent([uint32]$CallerId, [uint32]$CallerTid, [long]$CreationTime) {
    if ($env:GITHUB_ACTIONS -cne 'true' -or $env:GITHUB_WORKFLOW -cne 'Document Engine Development Checks' -or
        $env:OFOX_DOCUMENT_CHILD_BLOCK_EVENT -cne '1') { return 'query-unavailable' }
    $ErrorActionPreference = 'Stop'
    $channel = 'Microsoft-Windows-Security-Mitigations/KernelMode'
    try {
        if (-not (Get-WinEvent -ListLog $channel).IsEnabled) { return 'channel-disabled' }
        $filter = @{
            LogName = $channel
            ProviderName = 'Microsoft-Windows-Security-Mitigations'
            Id = 4
            StartTime = [datetime]::FromFileTimeUtc($CreationTime).ToString('o', [cultureinfo]::InvariantCulture)
            CallingProcessId = $CallerId.ToString()
        }
        $records = @(Get-WinEvent -FilterHashtable $filter -MaxEvents 33)
        if ($records.Count -gt 32) { return 'record-limit' }
        $matched = 0
        $requestUnverified = $false
        foreach ($record in $records) {
            $verdict = Get-DocumentChildEventMatch ([xml]$record.ToXml()) $CallerId $CallerTid $CreationTime
            if ($verdict -ceq 'matched') { $matched++ }
            if ($verdict -ceq 'request-unverified') { $requestUnverified = $true }
        }
        if ($matched -gt 1) { return 'ambiguous' }
        if ($matched -eq 1) { return 'matched' }
        if ($requestUnverified) { return 'request-unverified' }
        return 'no-match'
    } catch {
        if ($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound,*') { return 'no-match' }
        return 'query-unavailable'
    } finally {
        foreach ($record in $records) { $record.Dispose() }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    Read-DocumentChildBlockEvent $CallerId $CallerTid $CreationTime
}
