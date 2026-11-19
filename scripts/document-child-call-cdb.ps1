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
    try { Read-ChildCallRecord $State $Line } catch {
        if ($State.stage -in @('returned','done')) {
            if ($State.result -eq 'not-reached') { $script:failureStage = 'observation' }
            elseif ($_.FullyQualifiedErrorId -cin @('unpaired','invalid','sequence')) { $script:failureStage = 'return-conflict' }
            elseif ($_.FullyQualifiedErrorId -cin @('breakpoint','syntax','command-file')) {
                $phase = if ($State.stage -eq 'done') { 'after-exit-marker' } else { 'before-exit-marker' }
                $script:failureStage = 'return-error/' + $phase + '/' + $_.FullyQualifiedErrorId
            } else { $script:failureStage = 'return-observer-error' }
        }
        throw
    }
}

function Read-ChildCallRecord($State, [string]$Line) {
    if ($Line -match '^(?:\d+:)?\d+:[a-f0-9]+>\s*(.*)$') {
        $Line = $Matches[1]
        # Anchored markers exclude echoed commands; joined debugger errors still count.
    }
    # .catch prints the CDB error before its fixed fallback marker. Associate
    # that first error with the read already in progress and fail immediately.
    if ($Line -match 'unable to read command file|(?<syntax>syntax error)|(?<breakpoint>(?:unable|failed|could not|cannot) to (?:insert|set|resolve|read).*breakpoint|breakpoint.*(?:unresolved|could not be set))') {
        if ($State.stage -eq 'field') { $script:failureStage = 'symbols' }
        if ($State.stage -eq 'failed') { $script:failureStage = 'last-error-unreadable' }
        if ($Matches.ContainsKey('breakpoint')) { throw 'breakpoint' }
        if ($Matches.ContainsKey('syntax')) { throw 'syntax' }
        throw 'command-file'
    }
    if ($State.stage -eq 'setup' -and $Line -ceq 'OFOX_CALL FIELD_BEGIN') { $State.stage = 'field'; return }
    if ($State.stage -eq 'field' -and $Line -ceq 'OFOX_CALL FIELD_READY') { $State.stage = 'arm-ready'; return }
    if ($State.stage -eq 'field' -and $Line -ceq 'OFOX_CALL FIELD_UNAVAILABLE') {
        $script:failureStage = 'symbols'
        throw 'field_type'
    }
    if ($State.stage -eq 'failed' -and $Line -ceq 'OFOX_CALL READ_ERROR') {
        $script:failureStage = 'last-error-unreadable'
        throw 'field_read'
    }
    if ($Line -cmatch '^OFOX_CALL RETURN_WIN32 ([0-9a-fA-F]{8}) END$') {
        if ($State.stage -ne 'failed') { throw 'unpaired' }
        $code = [Convert]::ToUInt32($Matches[1],16)
        if ($code -eq 0) {
            $script:failureStage = 'last-error-zero'
            throw 'last_error'
        }
        $State.result = switch ($code) {
            367 { 'child-process-blocked' } 5 { 'access-denied' } 1260 { 'policy-disabled' }
            4252 { 'appcontainer-unsupported' } 1314 { 'privilege-not-held' }
            2 { 'file-not-found' } 3 { 'path-not-found' } 6 { 'invalid-handle' }
            default { 'other-win32' }
        }
        $State.stage = 'returned'
        return
    }
    if ($Line -match '^\s*LastErrorValue:' -or $Line -ceq 'OFOX_CALL INVALID') { throw 'invalid' }
    if (-not $Line.StartsWith('OFOX_CALL ')) { return }
    $stage = $State.stage
    if ($stage -eq 'arm-ready' -and $Line -ceq 'OFOX_CALL ARMED') { $State.stage = 'armed'; return }
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
    if ($stage -eq 'args' -and $Line -ceq 'OFOX_CALL RETURN_SUCCESS') { $State.stage = 'returned'; $State.result = 'succeeded'; return }
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

function Get-ChildCallProgress($State) {
    if ($State.stage -eq 'armed') { return 'ARMED' }
    if ($State.stage -eq 'returned') { return ('PAIRED ' + $State.result) }
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

function New-ChildCallToolStart([ValidateSet('symchk','cdb')][string]$Name, [string]$Dir) {
    $tool = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\Debuggers\x64\$Name.exe"
    $signature = Get-AuthenticodeSignature -LiteralPath $tool
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(?:^|,\s*)O=Microsoft Corporation(?:,|$)') { throw 'signature' }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $tool; $start.WorkingDirectory = $dir; $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true; $start.RedirectStandardInput = $true
    $start.CreateNoWindow = $true; $start.Environment.Clear()
    foreach ($key in @('SystemRoot','WINDIR')) { $start.Environment[$key] = $env:SystemRoot }
    $start.Environment['PATH'] = Join-Path $env:SystemRoot 'System32'
    foreach ($key in @('TEMP','TMP','USERPROFILE','APPDATA','LOCALAPPDATA')) { $start.Environment[$key] = $dir }
    return $start
}

function Write-ChildCallExitCommand([string]$Dir, [uint32]$ProcessId) {
    $command = '.if ((@$tpid == 0n{0}) and (@$t2 == 0) and (@$t3 == 0)) {{ .echo OFOX_CALL EXIT_NONE }} .elsif ((@$tpid == 0n{0}) and (@$t2 == 1) and (@$t3 == 1)) {{ .echo OFOX_CALL EXIT_PAIRED }} .else {{ .echo OFOX_CALL INVALID }}; q' -f $ProcessId
    # CDB's $>< converts a trailing newline into a semicolon, and q; is rejected.
    [IO.File]::WriteAllText((Join-Path $Dir 'exit.cmd'),$command,[Text.ASCIIEncoding]::new())
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
    $dir = [Environment]::CurrentDirectory
    $start = New-ChildCallToolStart 'cdb' $dir
    $script:failureStage = 'symbols'
    . (Join-Path $PSScriptRoot 'document-engine-symbols.ps1')
    $symbols = (New-Item -Path (Join-Path $dir 'symbols') -ItemType Directory -ErrorAction Stop).FullName
    $image = Join-Path $env:SystemRoot 'System32\ntdll.dll'
    $prepare = New-ChildCallToolStart 'symchk' $dir
    foreach ($arg in (Get-DocumentSymbolArguments $image $symbols)) { $prepare.ArgumentList.Add($arg) }
    # The host owns cancellation and reaps this controller's entire Job.
    Invoke-DocumentSymbolCheck $prepare ([Threading.Tasks.Task]::Delay(-1)) @{} 30000 $image

    # Use only the local symbol store during attachment. The live field size
    # and actual Node import must both resolve before the fixture is released.
    $script:failureStage = 'arm'
    $init = @(
        '.expr /s masm', 'sxi out', ('~~[0n' + $threadId + ']s'),
        # Require a matching PDB for ntdll while retaining kernel32 export lookup.
        '.echo OFOX_CALL FIELD_BEGIN', 'r @$t4 = 0', '.symopt+ 0x400',
        '.catch { .reload /f ntdll.dll; r @$t4 = @@c++(sizeof(@$teb->LastErrorValue)) }',
        '.symopt- 0x400', '.if (@$t4 == 4) { .echo OFOX_CALL FIELD_READY } .else { .echo OFOX_CALL FIELD_UNAVAILABLE }',
        'r @$t2 = 0', 'r @$t3 = 0',
        'bp10 kernel32!CreateProcessW "r @$t2 = @$t2 + 1; .echo OFOX_CALL ENTRY; $><entry.cmd"',
        'sxe -c "$><exit.cmd" epr',
        ('.if ((@$tpid == 0n{0}) and (@$tid == 0n{1}) and (@$bp10 != 0) and (@$bp10 == poi(node+0n{2})) and (@$t4 == 4)) {{ .echo OFOX_CALL ARMED }} .else {{ .echo OFOX_CALL INVALID }}' -f $processId,$threadId,$slot),
        'g'
    )
    $entry = @(
        ('.if ((@$tpid != 0n{0}) or (@$tid != 0n{1})) {{ .echo OFOX_CALL INVALID }}' -f $processId,$threadId),
        '.printf "OFOX_CALL IMAGE %mu\n", @rcx', '.printf "OFOX_CALL ARGS %mu\n", @rdx',
        'r @$t0 = @rsp', 'r @$t1 = poi(@rsp)', 'bp11 @$t1 "$><return.cmd"', 'gc'
    )
    # A failed expression otherwise stops the CDB command program. Mark the
    # DWORD valid only after evaluation, while the paired thread is stopped.
    $readError = @(
        '.echo OFOX_CALL FAILED', 'r @$t4 = 0',
        '.catch { r @$t5 = @@c++(@$teb->LastErrorValue); r @$t4 = 1 }',
        '.if (@$t4 == 1) { .printf "OFOX_CALL RETURN_WIN32 %08x END\n", @$t5 } .else { .echo OFOX_CALL READ_ERROR }'
    ) -join '; '
    # Count the matched return before printing its outcome. The outcome line
    # is the receipt for this stop; there is no trailing confirmation to lose.
    $ret = '.if ((@$tpid == 0n{0}) and (@$tid == 0n{1}) and (@rsp == (@$t0 + 8)) and (@rip == @$t1) and (@$t2 == 1) and (@$t3 == 0)) {{ r @$t3 = @$t3 + 1; .if (@eax == 0) {{ {2} }} .else {{ .printf "OFOX_CALL RETURN_SUCCESS\n" }} }} .else {{ .echo OFOX_CALL INVALID }}; bc 11; gc' -f $processId,$threadId,$readError
    foreach ($item in @{ 'init.cmd'=$init; 'entry.cmd'=$entry; 'return.cmd'=$ret }.GetEnumerator()) {
        [IO.File]::WriteAllLines((Join-Path $dir $item.Key),[string[]]$item.Value,[Text.ASCIIEncoding]::new())
    }
    Write-ChildCallExitCommand $dir $processId
    foreach ($arg in @('-hd','-sins','-netsyms:no','-noshell','-nosqm','-noinh','-y',("srv*" + $symbols),'-cf',(Join-Path $dir 'init.cmd'),'-p',[string]$processId)) { $start.ArgumentList.Add($arg) }
    $debug = [Diagnostics.Process]::Start($start)
    try {
        $state = New-ChildCallState $exe
        # Only stdout can supply protocol records. Any stderr makes the
        # observation inconclusive. The host owns the deadline and kill Job.
        $err = $debug.StandardError.ReadAsync([char[]]::new(1),0,1)
        $out = $debug.StandardOutput.ReadLineAsync()
        $total = 0
        while ($true) {
            if ($err.IsCompleted -and $err.GetAwaiter().GetResult() -ne 0) {
                if ($state.stage -in @('returned', 'done')) { $script:failureStage = 'exit-stderr' }
                throw 'debugger_error'
            }
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
                'field' { 'symbols' } 'arm-ready' { 'arm' }
                'armed' { 'observation' } 'entry' { 'request' } 'image' { 'request' }
                'args' { 'return-pair' } 'failed' { 'last-error' }
                'returned' { 'exit-marker' } default { 'exit' }
            }
            $progress = Get-ChildCallProgress $state
            if ($progress) { [Console]::Out.WriteLine($progress); [Console]::Out.Flush() }
        }
        if (-not $debug.WaitForExit(2000)) { $script:failureStage = 'exit-wait'; throw 'exit' }
        if ($debug.ExitCode -notin @(0,1)) { $script:failureStage = 'exit-status'; throw 'exit' }
        if ($err.GetAwaiter().GetResult() -ne 0) { $script:failureStage = 'exit-stderr'; throw 'exit' }
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
