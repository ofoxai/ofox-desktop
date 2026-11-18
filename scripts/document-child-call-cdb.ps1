# Private controller for the one checkpoint-7 request. No target, command, or
# output path can be supplied through workflow inputs or command-line options.
function New-ChildCallState([string]$Exe) {
    return @{ stage = 'setup'; exe = $Exe; result = $null }
}

function ConvertTo-ChildCallPath([string]$Value) {
    if ($Value.StartsWith('\\?\') -or $Value.StartsWith('\??\')) { return $Value.Substring(4) }
    return $Value
}

function Read-ChildCallLine($State, [string]$Line) {
    if ($Line -match '^(?:\d+:)?\d+:[a-f0-9]+>\s*(.*)$') {
        $Line = $Matches[1]
        # Anchored markers exclude echoed commands; joined debugger errors still count.
    }
    if ($Line -match '(?:unable|failed|could not|cannot) to (?:insert|set|resolve|read).*breakpoint|breakpoint.*(?:unresolved|could not be set)|syntax error|unable to read command file') { throw 'debugger' }
    if ($Line -cmatch '^\s*LastErrorValue:\s*\(Win32\)\s*(?:0x([0-9a-fA-F]{1,8})|0n([0-9]{1,10})|([0-9]{1,10}))(?:\s+.*)?$') {
        if ($State.stage -ne 'failed') { throw 'unpaired' }
        $code = if ($Matches[1]) { [Convert]::ToUInt32($Matches[1],16) } else { [uint32]($Matches[2] + $Matches[3]) }
        if ($Line -match '\s+\((\d+)\)' -and $code -ne [uint32]$Matches[1]) { throw 'last_error' }
        if ($code -eq 0) { throw 'last_error' }
        $State.result = switch ($code) {
            367 { 'child-process-blocked' } 5 { 'access-denied' } 1260 { 'policy-disabled' }
            4252 { 'appcontainer-unsupported' } 1314 { 'privilege-not-held' }
            2 { 'file-not-found' } 3 { 'path-not-found' } 6 { 'invalid-handle' }
            default { 'other-win32' }
        }
        $State.stage = 'result'
        return
    }
    if ($Line -match '^\s*LastErrorValue:' -or $Line -ceq 'OFOX_CALL INVALID') { throw 'invalid' }
    if (-not $Line.StartsWith('OFOX_CALL ')) { return }
    $stage = $State.stage
    if ($stage -eq 'setup' -and $Line -ceq 'OFOX_CALL ARMED') { $State.stage = 'armed'; return }
    if ($stage -eq 'armed' -and $Line -ceq 'OFOX_CALL ENTRY') { $State.stage = 'entry'; return }
    if ($stage -eq 'entry' -and $Line.StartsWith('OFOX_CALL IMAGE ')) {
        $image = ConvertTo-ChildCallPath $Line.Substring(16)
        if (-not [string]::Equals($image,$State.exe,[StringComparison]::OrdinalIgnoreCase)) { throw 'image' }
        $State.stage = 'image'; return
    }
    if ($stage -eq 'image' -and $Line.StartsWith('OFOX_CALL ARGS ')) {
        $cmd = $Line.Substring(15)
        $suffix = ' -e "require(''node:fs'').writeFileSync(''child-marker'', ''bad'')"'
        if (-not $cmd.EndsWith($suffix,[StringComparison]::Ordinal)) { throw 'request' }
        $argv0 = $cmd.Substring(0,$cmd.Length - $suffix.Length)
        if ($argv0 -cmatch '^"([^"]+)"$') { $argv0 = $Matches[1] }
        elseif ($argv0 -match '[\s"]') { throw 'argv0' }
        if (-not [string]::Equals((ConvertTo-ChildCallPath $argv0),$State.exe,[StringComparison]::OrdinalIgnoreCase)) { throw 'argv0' }
        $State.stage = 'args'; return
    }
    if ($stage -eq 'args' -and $Line -ceq 'OFOX_CALL FAILED') { $State.stage = 'failed'; return }
    if ($stage -eq 'args' -and $Line -ceq 'OFOX_CALL SUCCEEDED') { $State.stage = 'result'; $State.result = 'succeeded'; return }
    if ($stage -eq 'result' -and $Line -ceq 'OFOX_CALL RETURN') { $State.stage = 'returned'; return }
    if (($stage -eq 'armed' -and $Line -ceq 'OFOX_CALL EXIT_NONE') -or
        ($stage -eq 'returned' -and $Line -ceq 'OFOX_CALL EXIT_PAIRED')) {
        if ($stage -eq 'armed') { $State.result = 'not-reached' }
        $State.stage = 'done'; return
    }
    throw 'sequence'
}

function Complete-ChildCall($State) {
    if ($State.stage -ne 'done' -or -not $State.result) { throw 'incomplete' }
    return $State.result
}

# Locate the fixed API in the pinned image's PE32+ import table. The live
# debugger must prove its breakpoint equals the address in this exact IAT slot.
function Get-ChildCallImport([string]$Exe) {
    $file = [IO.File]::OpenRead($Exe)
    try {
        $pe = [Reflection.PortableExecutable.PEReader]::new($file)
        try {
            if ($pe.PEHeaders.CoffHeader.Machine -ne 'Amd64' -or $pe.PEHeaders.PEHeader.Magic -ne 'PE32Plus') { throw 'architecture' }
            $dir = $pe.PEHeaders.PEHeader.ImportTableDirectory
            $descriptors = $pe.GetSectionData($dir.RelativeVirtualAddress).GetReader()
            $slot = 0
            for ($i = 0; $i -lt [Math]::Min(256,[Math]::Floor($dir.Size / 20)); $i++) {
                $names = $descriptors.ReadInt32()
                $null = $descriptors.ReadInt64()
                $dll = $descriptors.ReadInt32()
                $iat = $descriptors.ReadInt32()
                if ($dll -eq 0) {
                    if ($slot -gt 0) { return $slot }
                    break
                }
                $name = $pe.GetSectionData($dll).GetReader()
                if ($name.RemainingBytes -lt 13 -or -not [string]::Equals($name.ReadUTF8(13),"KERNEL32.DLL`0",[StringComparison]::OrdinalIgnoreCase)) { continue }
                $table = $pe.GetSectionData($names).GetReader()
                for ($j = 0; $j -lt 4096; $j++) {
                    $rva = $table.ReadInt64()
                    if ($rva -eq 0) { break }
                    if ($rva -lt 0) { continue } # ordinal import
                    $entry = $pe.GetSectionData([int]$rva + 2).GetReader()
                    if ($entry.RemainingBytes -ge 15 -and [string]::Equals($entry.ReadUTF8(15),"CreateProcessW`0",[StringComparison]::Ordinal)) {
                        if ($slot -ne 0) { throw 'duplicate_import' }
                        $slot = $iat + 8 * $j
                    }
                }
            }
            throw 'missing_import'
        } finally { $pe.Dispose() }
    } finally { $file.Dispose() }
}

function Invoke-FixedChildCall {
    if (-not $IsWindows -or -not [Environment]::Is64BitProcess -or
        $env:GITHUB_ACTIONS -cne 'true' -or $env:GITHUB_WORKFLOW -cne 'Document Engine Child Call Diagnostic' -or
        $env:GITHUB_REF -cne 'refs/heads/codex/desktop-next' -or $env:GITHUB_EVENT_NAME -cnotin @('push','workflow_dispatch') -or $env:OFOX_DOCUMENT_CHILD_CALL_TRACE -cne '1' -or
        $env:OFOX_DOCUMENT_STARTUP_TRACE -or $env:OFOX_DOCUMENT_STARTUP_COMPARISON -or $env:OFOX_DOCUMENT_CDB_DIAGNOSTIC_ROOT) { throw 'context' }
    $script:failureStage = 'target'
    $identity = [Console]::In.ReadLine() | ConvertFrom-Json
    $processId = [uint32]$identity.process; $threadId = [uint32]$identity.thread
    if ($processId -eq 0 -or $threadId -eq 0) { throw 'identity' }
    $target = Get-CimInstance Win32_Process -Filter "ProcessId=$processId"
    $controller = Get-CimInstance Win32_Process -Filter "ProcessId=$PID"
    $exe = ConvertTo-ChildCallPath $identity.exe
    if ($target.ParentProcessId -ne $identity.host -or $controller.ParentProcessId -ne $identity.host -or
        $target.Name -ine 'node.exe' -or -not [string]::Equals((ConvertTo-ChildCallPath $target.ExecutablePath),$exe,[StringComparison]::OrdinalIgnoreCase) -or
        (Get-Process -Id $processId).Threads.Id -notcontains $threadId) { throw 'identity' }
    if ($target.CommandLine -cnotmatch 'OFOX_CHILD_BEGIN' -or $target.CommandLine -cnotmatch 'OFOX_LPAC_CHILD_RESULT:') { throw 'fixture' }
    $script:failureStage = 'import'
    $slot = Get-ChildCallImport $exe
    $script:failureStage = 'arm'
    $tool = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Debuggers\x64\cdb.exe'
    $signature = Get-AuthenticodeSignature -LiteralPath $tool
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)') { throw 'signature' }

    # Current directory is the Rust host's fresh TempDir outside the artifact
    # tree. Only fixed command files live here. No transcript is written.
    $dir = [Environment]::CurrentDirectory
    $init = @(
        '.expr /s masm', 'sxi out', ('~~[0n' + $threadId + ']s'),
        'r @$t2 = 0', 'r @$t3 = 0',
        'bp10 kernel32!CreateProcessW "r @$t2 = @$t2 + 1; .echo OFOX_CALL ENTRY; $><entry.cmd"',
        'sxe -c "$><exit.cmd" epr',
        ('.if ((@$tpid == 0n{0}) and (@$tid == 0n{1}) and (@$bp10 != 0) and (@$bp10 == poi(node+0n{2}))) {{ .echo OFOX_CALL ARMED }} .else {{ .echo OFOX_CALL INVALID }}' -f $processId,$threadId,$slot),
        'g'
    )
    $entry = @(
        ('.if ((@$tpid != 0n{0}) or (@$tid != 0n{1})) {{ .echo OFOX_CALL INVALID }}' -f $processId,$threadId),
        '.printf "OFOX_CALL IMAGE %mu\n", @rcx', '.printf "OFOX_CALL ARGS %mu\n", @rdx',
        'r @$t0 = @rsp', 'r @$t1 = poi(@rsp)', 'bp11 @$t1 "$><return.cmd"', 'gc'
    )
    $ret = '.if ((@$tpid == 0n{0}) and (@$tid == 0n{1}) and (@rsp == (@$t0 + 8)) and (@rip == @$t1)) {{ .if (@eax == 0) {{ .echo OFOX_CALL FAILED; !gle }} .else {{ .echo OFOX_CALL SUCCEEDED }}; r @$t3 = @$t3 + 1; .echo OFOX_CALL RETURN }} .else {{ .echo OFOX_CALL INVALID }}; bc 11; gc' -f $processId,$threadId
    $end = '.if ((@$tpid == 0n{0}) and (@$t2 == 0) and (@$t3 == 0)) {{ .echo OFOX_CALL EXIT_NONE }} .elsif ((@$tpid == 0n{0}) and (@$t2 == 1) and (@$t3 == 1)) {{ .echo OFOX_CALL EXIT_PAIRED }} .else {{ .echo OFOX_CALL INVALID }}; q' -f $processId
    foreach ($item in @{ 'init.cmd'=$init; 'entry.cmd'=$entry; 'return.cmd'=$ret; 'exit.cmd'=$end }.GetEnumerator()) {
        [IO.File]::WriteAllLines((Join-Path $dir $item.Key),[string[]]$item.Value,[Text.ASCIIEncoding]::new())
    }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $tool; $start.WorkingDirectory = $dir; $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true; $start.RedirectStandardInput = $true
    $start.CreateNoWindow = $true; $start.Environment.Clear()
    foreach ($key in @('SystemRoot','WINDIR')) { $start.Environment[$key] = $env:SystemRoot }
    $start.Environment['PATH'] = Join-Path $env:SystemRoot 'System32'
    foreach ($key in @('TEMP','TMP','USERPROFILE','APPDATA','LOCALAPPDATA')) { $start.Environment[$key] = $dir }
    foreach ($arg in @('-hd','-sins','-netsyms:no','-noshell','-nosqm','-noinh','-y',$dir,'-cf',(Join-Path $dir 'init.cmd'),'-p',[string]$processId)) { $start.ArgumentList.Add($arg) }
    $debug = [Diagnostics.Process]::Start($start)
    try {
        $state = New-ChildCallState $exe
        # Only stdout can supply protocol records. Any stderr makes the
        # observation inconclusive. The host owns the deadline and kill Job.
        $err = $debug.StandardError.ReadAsync([char[]]::new(1),0,1)
        $out = $debug.StandardOutput.ReadLineAsync()
        $total = 0
        while ($true) {
            if ($err.IsCompleted -and $err.GetAwaiter().GetResult() -ne 0) { throw 'debugger_error' }
            if (-not $out.Wait(50)) { continue }
            $line = $out.GetAwaiter().GetResult()
            if ($null -eq $line) { break }
            $total += $line.Length + 1
            if ($total -gt 262144 -or $line.Length -gt 8192) { throw 'bound' }
            $out = $debug.StandardOutput.ReadLineAsync()
            $before = $state.stage
            Read-ChildCallLine $state $line
            if ($state.stage -eq $before) { continue }
            $script:failureStage = switch ($state.stage) {
                'armed' { 'observation' } 'entry' { 'request' } 'image' { 'request' }
                'args' { 'return-pair' } 'failed' { 'last-error' } 'result' { 'return-pair' }
                default { 'exit' }
            }
            if ($state.stage -eq 'armed') { [Console]::Out.WriteLine('ARMED'); [Console]::Out.Flush() }
        }
        if (-not $debug.WaitForExit(2000) -or $debug.ExitCode -notin @(0,1) -or $err.GetAwaiter().GetResult() -ne 0) { throw 'exit' }
        [Console]::Out.WriteLine((Complete-ChildCall $state))
    } finally {
        if (-not $debug.HasExited) { $debug.Kill($true); $null = $debug.WaitForExit(2000) }
        $debug.Dispose()
    }
}
if ($MyInvocation.InvocationName -ne '.') {
    $ErrorActionPreference = 'Stop'
    $script:failureStage = 'target'
    try { Invoke-FixedChildCall } catch { [Console]::Out.WriteLine('INCOMPLETE ' + $script:failureStage); exit 1 }
}
