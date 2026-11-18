# Test-only fixed observers for the first trusted Node startup. No target policy,
# token changes, target function calls, or arbitrary debugger commands.
function Get-DocumentOperationDefinitions {
  return @(
    @{ Name = 'NtOpenKey'; Kind = 'attributes' },
    @{ Name = 'NtOpenKeyEx'; Kind = 'attributes' },
    @{ Name = 'NtCreateKey'; Kind = 'attributes' },
    @{ Name = 'NtQueryValueKey'; Kind = 'value' },
    @{ Name = 'NtOpenSection'; Kind = 'attributes' },
    @{ Name = 'NtOpenFile'; Kind = 'attributes' },
    @{ Name = 'NtCreateFile'; Kind = 'attributes' },
    @{ Name = 'NtOpenDirectoryObject'; Kind = 'attributes' },
    @{ Name = 'NtOpenEvent'; Kind = 'attributes' },
    @{ Name = 'NtAlpcConnectPort'; Kind = 'port' },
    @{ Name = 'NtConnectPort'; Kind = 'none' },
    @{ Name = 'NtMapViewOfSection'; Kind = 'none' },
    @{ Name = 'NtAlpcConnectPortEx'; Kind = 'none' },
    @{ Name = 'NtSecureConnectPort'; Kind = 'none' },
    # Status only: KernelBase launches conhost for a new console during attach.
    @{ Name = 'NtCreateUserProcess'; Kind = 'none' },
    # Status only: key operations on an already open handle (no name argument).
    @{ Name = 'NtEnumerateKey'; Kind = 'none' },
    @{ Name = 'NtQueryKey'; Kind = 'none' },
    @{ Name = 'NtNotifyChangeKey'; Kind = 'none' },
    @{ Name = 'NtNotifyChangeMultipleKeys'; Kind = 'none' }
  )
}
function New-DocumentOperationSetup([uint32]$PidValue, [uint32]$TidValue) {
  if ($PidValue -eq 0 -or $TidValue -eq 0) { throw 'operation_target_invalid' }
  return @{ pidValue = $PidValue; tidValue = $TidValue; threadIndex = $null; section = ''; abiVerified = $false
    threadsDone = $false; breakpointsDone = $false; rows = @{} }
}
function Confirm-DocumentOperationArchitecture([hashtable]$State, $Architecture) {
  if ($null -eq $Architecture -or $Architecture.apiSucceeded -isnot [bool] -or -not $Architecture.apiSucceeded -or
      $Architecture.compiledLayoutVerified -isnot [bool] -or -not $Architecture.compiledLayoutVerified -or
      ($Architecture.processMachine -isnot [int] -and $Architecture.processMachine -isnot [long]) -or
      ($Architecture.nativeMachine -isnot [int] -and $Architecture.nativeMachine -isnot [long]) -or
      $Architecture.processMachine -ne 0 -or $Architecture.nativeMachine -ne 0x8664) { throw 'operation_architecture_unverified' }
  $State.abiVerified = $true
}
function Get-DocumentOperationProgram([uint32]$TidValue) {
  if ($TidValue -eq 0) { throw 'operation_target_invalid' }
  $thread = '~~[0n' + $TidValue + ']'
  $files = @{}
  $commands = @(
    '.expr /s masm',
    # Run 44221172203 froze after DLL mapping: attaching sets BeingDebugged, so
    # the loader executes its debugger break (int3) after the static imports
    # are mapped and before any DllMain. Report that and every exception
    # without stopping, each keeping its default handling (breakpoints:
    # handled; others: passed to the target). Only the invalid-handle exception
    # NtClose raises solely because a debugger is attached is set to handled,
    # matching an undebugged run. Engine breakpoints (bp10-32) are unaffected.
    # Run 44226276948: once startup proceeded, loader-snap debuggee output
    # overflowed the private log. The snaps flag and its readback stay; only
    # the debuggee-output event (not an exception) is no longer captured.
    'sxi out',
    'sxn ibp', 'sxn bpe', 'sxe -h ch', 'sxn ch', 'sxn av', 'sxn dm', 'sxn dz', 'sxn c000008e', 'sxn gp',
    'sxn iov', 'sxn ip', 'sxn isc', 'sxn lsq', 'sxn sbo', 'sxn sov', 'sxn asrt', 'sxn wkd', 'sxn aph',
    'sxn eh', 'sxn ii', 'sxn *',
    '.echo OFOX_NT_RAW_V1 THREADS_BEGIN', '~', '.echo OFOX_NT_RAW_V1 THREADS_END',
    'r @$t0 = 0', 'r @$t1 = 0', 'r @$t10 = 0'
  )
  # Persistent owned return ID: a wrong stack cannot consume it. Only the exact
  # saved call's return clears it. gc is confined to our observational BPs;
  # genuine exceptions retain the debugger's ordinary stop behavior.
  $returned = '.if ((@$tid == 0n' + $TidValue + ') and (@rsp == (@$t2 + 8)) and (@rip == @$t3)) { .printf "OFOX_NT_RAW_V1 RETURN seq=%d pid=%x tid=%x sp=%I64x status=%x END\n", @$t0, @$tpid, @$tid, @rsp, @eax; bc 30; r @$t1 = 0; gc } .else { .echo OFOX_NT_RAW_V1 INVALID return_pair END; gc }'
  $files['operation-return.cmd'] = $returned
  $definitions = @(Get-DocumentOperationDefinitions)
  # IDs 30-32 belong to the return and scope breakpoints.
  if (9 + $definitions.Count -ge 30) { throw 'operation_install_unverified' }
  $range = '10-' + (9 + $definitions.Count)
  for ($index = 0; $index -lt $definitions.Count; $index++) {
    $definition = $definitions[$index]; $id = 10 + $index; $api = $index + 1
    $body = @('r @$t1 = 1', 'r @$t0 = @$t0 + 1', 'r @$t2 = @rsp', 'r @$t3 = poi(@rsp)', 'r @$t5 = 0', 'r @$t6 = 2')
    $access = 'none'; $root = 'none'; $arguments = ''
    if ($definition.Kind -eq 'attributes') {
      # Published Windows x64 ABI, bound to the actual child architecture.
      # Optional unreadable/mismatched attributes yield unknown, not a fake name.
      $body += '.if (@r8 != 0) { .if ($vvalid(@r8, 0x30)) { .if (dwo(@r8) == 0x30) { r @$t5 = poi(@r8 + 0x10); r @$t6 = (poi(@r8 + 8) == 0) } } }'
      $access = '%x'; $root = '%x'; $arguments = ', @edx, @$t6'
    } elseif ($definition.Kind -eq 'port') {
      # Microsoft Terminal's NtAlpcConnectPort call supplies PortName in RDX.
      # RDX is not an ACCESS_MASK. ConnectPort without this contract is status-only.
      $body += 'r @$t5 = @rdx'; $root = '1'
    } elseif ($definition.Kind -eq 'value') { $body += 'r @$t5 = @rdx' }
    $body += '.printf "OFOX_NT_RAW_V1 ENTER seq=%d api=' + $api + ' pid=%x tid=%x sp=%I64x ret=%I64x access=' + $access + ' root=' + $root + ' END\n", @$t0, @$tpid, @$tid, @$t2, @$t3' + $arguments
    if ($definition.Kind -eq 'none') {
      $body += '.printf "OFOX_NT_RAW_V1 NAME_UNKNOWN seq=%d END\n", @$t0'
    } else {
      # %msu uses UNICODE_STRING.Length. Never scan for a NUL past the bound.
      $body += 'r @$t7 = 0; .if (@$t5 != 0) { .if ($vvalid(@$t5, 0x10)) { .if ((wo(@$t5) <= 0x400) and ((wo(@$t5) & 1) == 0) and (wo(@$t5) <= wo(@$t5 + 2))) { .if (wo(@$t5) == 0) { r @$t7 = 2 } .else { .if (poi(@$t5 + 8) != 0) { .if ($vvalid(poi(@$t5 + 8), wo(@$t5))) { r @$t7 = 1 } } } } } }; .if (@$t7 == 2) { .printf "OFOX_NT_RAW_V1 NAME seq=%d bytes=0 value=\"\" END\n", @$t0 } .elsif (@$t7 == 1) { .printf "OFOX_NT_RAW_V1 NAME seq=%d bytes=%d value=\"%msu\" END\n", @$t0, wo(@$t5), @$t5 } .else { .printf "OFOX_NT_RAW_V1 NAME_UNKNOWN seq=%d END\n", @$t0 }'
    }
    $body += $thread + 'bp30 @$t3 "$><operation-return.cmd"'
    $body += '.printf "OFOX_NT_RAW_V1 RETURN_BP_BEGIN seq=%d END\n", @$t0'
    $body += 'bl /L 30'
    $body += '.printf "OFOX_NT_RAW_V1 RETURN_BP_END seq=%d END\n", @$t0'
    $body += 'gc'
    $entry = '.if (@$t1 != 0) { .echo OFOX_NT_RAW_V1 INVALID nested END; gc } .elsif (@$t0 >= 0n1024) { .echo OFOX_NT_RAW_V1 INVALID limit END; bd ' + $range + '; gc } .else { ' + ($body -join '; ') + ' }'
    # Fixed relative script names in the owned private working directory avoid
    # nested CommandString escaping. $>< executes the file as one command block.
    $files['operation-' + $api + '.cmd'] = $entry
    $commands += $thread + 'bp' + $id + ' ntdll!' + $definition.Name + ' "$><operation-' + $api + '.cmd"'
  }
  # Best-effort scope: libuv's ws2_32!WSAStartup on the same thread, deferred
  # until the module loads. It is set after the verified listing and never
  # gates the trace; seq is the number of observed calls entered so far. Only
  # the exact saved return clears it, and only the first call is scoped.
  $files['operation-scope.cmd'] = '.if (@$tid != 0n' + $TidValue + ') { gc } .elsif (@$t10 != 0) { .echo OFOX_NT_RAW_V1 INVALID scope END; gc } .else { r @$t10 = 1; r @$t11 = @rsp; r @$t12 = poi(@rsp); .printf "OFOX_NT_RAW_V1 SCOPE_ENTER seq=%d pid=%x tid=%x sp=%I64x ret=%I64x END\n", @$t0, @$tpid, @$tid, @$t11, @$t12; ' + $thread + 'bp32 @$t12 "$><operation-scope-return.cmd"; gc }'
  $files['operation-scope-return.cmd'] = '.if ((@$tid == 0n' + $TidValue + ') and (@rsp == (@$t11 + 8)) and (@rip == @$t12)) { .printf "OFOX_NT_RAW_V1 SCOPE_RETURN seq=%d pid=%x tid=%x sp=%I64x result=%x END\n", @$t0, @$tpid, @$tid, @rsp, @eax; bc 31; bc 32; r @$t10 = 2; gc } .else { .echo OFOX_NT_RAW_V1 INVALID scope_pair END; gc }'
  return @{ commands = $commands + @('.echo OFOX_NT_RAW_V1 BREAKPOINTS_BEGIN', ('bl /L ' + $range), '.echo OFOX_NT_RAW_V1 BREAKPOINTS_END',
      ($thread + 'bu31 ws2_32!WSAStartup "$><operation-scope.cmd"')); files = $files }
}
function Read-DocumentOperationSetupLine([hashtable]$State, [string]$Line) {
  if ($Line -eq 'OFOX_NT_RAW_V1 THREADS_BEGIN') {
    if (-not $State.abiVerified -or $State.threadsDone -or $State.section) { throw 'operation_thread_unverified' }; $State.section = 'threads'
  } elseif ($Line -eq 'OFOX_NT_RAW_V1 THREADS_END') {
    if ($State.section -ne 'threads' -or $null -eq $State.threadIndex) { throw 'operation_thread_unverified' }; $State.section = ''; $State.threadsDone = $true
  } elseif ($Line -eq 'OFOX_NT_RAW_V1 BREAKPOINTS_BEGIN') {
    if (-not $State.threadsDone -or $State.breakpointsDone -or $State.section) { throw 'operation_install_unverified' }; $State.section = 'breakpoints'
  } elseif ($Line -eq 'OFOX_NT_RAW_V1 BREAKPOINTS_END') {
    if ($State.section -ne 'breakpoints' -or $State.rows.Count -ne @(Get-DocumentOperationDefinitions).Count) { throw 'operation_install_unverified' }; $State.section = ''; $State.breakpointsDone = $true
  } elseif ($State.section -eq 'threads') {
    if ($Line -match '^\s*[.#]?\s*(\d{1,3})\s+Id:\s+([a-fA-F0-9]{1,8})\.([a-fA-F0-9]{1,8})\s+Suspend:') {
      if ([Convert]::ToUInt32($Matches[2], 16) -eq $State.pidValue -and [Convert]::ToUInt32($Matches[3], 16) -eq $State.tidValue) {
        if ($null -ne $State.threadIndex) { throw 'operation_thread_unverified' }; $State.threadIndex = [int]$Matches[1]
      }
    }
  } elseif ($State.section -eq 'breakpoints' -and $Line.Trim() -and $Line -notmatch '^(?:\d{1,3}:)?\d{1,3}:[a-fA-F0-9]{3}>') {
    # CDB echoes each -cf command after its prompt; an echo is never a row.
    # Current dbgeng prints DML action text after the state and may qualify a
    # thread-specific column with ~. Only enabled rows on the exact thread pass.
    if ($Line -notmatch '^\s*(\d{2}) e (?:Disable Clear\s+)?(?:[a-fA-F0-9]{8}`[a-fA-F0-9]{8}|[a-fA-F0-9]{8,16})\s+0*1\s+\(0*1\)\s+0:~?(\d{3})\s+ntdll!((?:Nt|Zw)[A-Za-z]+)(?:\+0x0+)?(?:\s.*)?$') { throw 'operation_install_unverified' }
    $id = [int]$Matches[1]; $threadIndex = [int]$Matches[2]; $name = $Matches[3]
    $definitions = @(Get-DocumentOperationDefinitions)
    if ($id -lt 10 -or $id -ge 10 + $definitions.Count) { throw 'operation_install_unverified' }
    $expected = $definitions[$id - 10].Name
    if ($threadIndex -ne $State.threadIndex -or $State.rows.ContainsKey($id) -or $name.Substring(2) -ne $expected.Substring(2)) { throw 'operation_install_unverified' }
    $State.rows[$id] = $true
  }
}
function Assert-DocumentOperationSetup([hashtable]$State) {
  if (-not $State.abiVerified -or -not $State.threadsDone -or -not $State.breakpointsDone -or $State.section -or $State.rows.Count -ne @(Get-DocumentOperationDefinitions).Count) { throw 'operation_install_unverified' }
}
function Get-DocumentOperationSetupState($State) {
  return @{ abiVerified = ($null -ne $State -and $State.abiVerified -eq $true)
    threadMatched = ($null -ne $State -and $State.threadsDone -eq $true)
    breakpointRows = $(if ($null -eq $State) { 0 } else { $State.rows.Count })
    installationVerified = ($null -ne $State -and $State.breakpointsDone -eq $true) }
}
