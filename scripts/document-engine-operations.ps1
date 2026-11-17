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
    @{ Name = 'NtSecureConnectPort'; Kind = 'none' }
  )
}
function New-DocumentOperationSetup([uint32]$PidValue, [uint32]$TidValue) {
  if ($PidValue -eq 0 -or $TidValue -eq 0) { throw 'operation_target_invalid' }
  return @{ pidValue = $PidValue; tidValue = $TidValue; threadIndex = $null; section = ''; layout = $false
    threadsDone = $false; breakpointsDone = $false; rows = @{} }
}
function Get-DocumentOperationProgram([uint32]$TidValue) {
  if ($TidValue -eq 0) { throw 'operation_target_invalid' }
  $thread = '~~[0n' + $TidValue + ']'
  $files = @{}
  $commands = @(
    '.printf "OFOX_NT_RAW_V1 LAYOUT oa=%x root=%x name=%x us=%x length=%x buffer=%x END\n", @@c++(sizeof(ntdll!_OBJECT_ATTRIBUTES)), @@c++((unsigned __int64)&((ntdll!_OBJECT_ATTRIBUTES*)0)->RootDirectory), @@c++((unsigned __int64)&((ntdll!_OBJECT_ATTRIBUTES*)0)->ObjectName), @@c++(sizeof(ntdll!_UNICODE_STRING)), @@c++((unsigned __int64)&((ntdll!_UNICODE_STRING*)0)->Length), @@c++((unsigned __int64)&((ntdll!_UNICODE_STRING*)0)->Buffer)',
    '.echo OFOX_NT_RAW_V1 THREADS_BEGIN', '~', '.echo OFOX_NT_RAW_V1 THREADS_END',
    'r @$t0 = 0', 'r @$t1 = 0'
  )
  # Persistent owned return ID: a wrong stack cannot consume it. Only the exact
  # saved call's return clears it. gc is confined to our observational BPs;
  # genuine exceptions retain the debugger's ordinary stop behavior.
  $returned = '.if ((@$tid == 0n' + $TidValue + ') && (@rsp == (@$t2 + 8)) && (@rip == @$t3)) { .printf "OFOX_NT_RAW_V1 RETURN seq=%d pid=%x tid=%x sp=%I64x status=%x END\n", @$t0, @$tpid, @$tid, @rsp, @eax; bc 30; r @$t1 = 0; gc } .else { .echo OFOX_NT_RAW_V1 INVALID return_pair END; gc }'
  $files['operation-return.cmd'] = $returned
  $definitions = @(Get-DocumentOperationDefinitions)
  for ($index = 0; $index -lt $definitions.Count; $index++) {
    $definition = $definitions[$index]; $id = 10 + $index; $api = $index + 1
    $body = @('r @$t1 = 1', 'r @$t0 = @$t0 + 1', 'r @$t2 = @rsp', 'r @$t3 = poi(@rsp)', 'r @$t5 = 0', 'r @$t6 = 0')
    $access = 'none'; $root = 'none'; $arguments = ''
    if ($definition.Kind -eq 'attributes') {
      # x64 layout is checked against the actual matched PDB before READY.
      $body += '.if (@r8 != 0) { r @$t5 = poi(@r8 + 0x10); r @$t6 = (poi(@r8 + 8) == 0) }'
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
      $body += '.if (@$t5 != 0) { .if ((wo(@$t5) <= 0x400) && ((wo(@$t5) & 1) == 0) && (wo(@$t5) <= wo(@$t5 + 2)) && ((wo(@$t5) == 0) || (poi(@$t5 + 8) != 0))) { .printf "OFOX_NT_RAW_V1 NAME seq=%d bytes=%d value=\"%msu\" END\n", @$t0, wo(@$t5), @$t5 } .else { .printf "OFOX_NT_RAW_V1 NAME_UNKNOWN seq=%d END\n", @$t0 } } .else { .printf "OFOX_NT_RAW_V1 NAME_UNKNOWN seq=%d END\n", @$t0 }'
    }
    $body += $thread + 'bp30 @$t3 "$><operation-return.cmd"'
    $body += '.printf "OFOX_NT_RAW_V1 RETURN_BP_BEGIN seq=%d END\n", @$t0'
    $body += 'bl /L 30'
    $body += '.printf "OFOX_NT_RAW_V1 RETURN_BP_END seq=%d END\n", @$t0'
    $body += 'gc'
    $entry = '.if (@$t1 != 0) { .echo OFOX_NT_RAW_V1 INVALID nested END; gc } .elsif (@$t0 >= 0n256) { .echo OFOX_NT_RAW_V1 INVALID limit END; bd 10-23; gc } .else { ' + ($body -join '; ') + ' }'
    # Fixed relative script names in the owned private working directory avoid
    # nested CommandString escaping. $>< executes the file as one command block.
    $files['operation-' + $api + '.cmd'] = $entry
    $commands += $thread + 'bp' + $id + ' ntdll!' + $definition.Name + ' "$><operation-' + $api + '.cmd"'
  }
  return @{ commands = $commands + @('.echo OFOX_NT_RAW_V1 BREAKPOINTS_BEGIN', 'bl /L 10-23', '.echo OFOX_NT_RAW_V1 BREAKPOINTS_END'); files = $files }
}
function Read-DocumentOperationSetupLine([hashtable]$State, [string]$Line) {
  if ($Line -match '^OFOX_NT_RAW_V1 LAYOUT ') {
    if ($State.layout -or $Line -ne 'OFOX_NT_RAW_V1 LAYOUT oa=30 root=8 name=10 us=10 length=0 buffer=8 END') { throw 'operation_layout_unverified' }
    $State.layout = $true
  } elseif ($Line -eq 'OFOX_NT_RAW_V1 THREADS_BEGIN') {
    if (-not $State.layout -or $State.threadsDone -or $State.section) { throw 'operation_thread_unverified' }; $State.section = 'threads'
  } elseif ($Line -eq 'OFOX_NT_RAW_V1 THREADS_END') {
    if ($State.section -ne 'threads' -or $null -eq $State.threadIndex) { throw 'operation_thread_unverified' }; $State.section = ''; $State.threadsDone = $true
  } elseif ($Line -eq 'OFOX_NT_RAW_V1 BREAKPOINTS_BEGIN') {
    if (-not $State.threadsDone -or $State.breakpointsDone -or $State.section) { throw 'operation_install_unverified' }; $State.section = 'breakpoints'
  } elseif ($Line -eq 'OFOX_NT_RAW_V1 BREAKPOINTS_END') {
    if ($State.section -ne 'breakpoints' -or $State.rows.Count -ne 14) { throw 'operation_install_unverified' }; $State.section = ''; $State.breakpointsDone = $true
  } elseif ($State.section -eq 'threads') {
    if ($Line -match '^\s*[.#]?\s*(\d{1,3})\s+Id:\s+([a-fA-F0-9]{1,8})\.([a-fA-F0-9]{1,8})\s+Suspend:') {
      if ([Convert]::ToUInt32($Matches[2], 16) -eq $State.pidValue -and [Convert]::ToUInt32($Matches[3], 16) -eq $State.tidValue) {
        if ($null -ne $State.threadIndex) { throw 'operation_thread_unverified' }; $State.threadIndex = [int]$Matches[1]
      }
    }
  } elseif ($State.section -eq 'breakpoints' -and $Line.Trim()) {
    if ($Line -notmatch '^\s*(1[0-9]|2[0-3]) e (?:[a-fA-F0-9]{8}`[a-fA-F0-9]{8}|[a-fA-F0-9]{8,16})\s+0*1\s+\(0*1\)\s+0:(\d{3})\s+ntdll!((?:Nt|Zw)[A-Za-z]+)(?:\+0x0+)?(?:\s.*)?$') { throw 'operation_install_unverified' }
    $id = [int]$Matches[1]; $threadIndex = [int]$Matches[2]; $name = $Matches[3]
    $expected = @(Get-DocumentOperationDefinitions)[$id - 10].Name
    if ($threadIndex -ne $State.threadIndex -or $State.rows.ContainsKey($id) -or $name.Substring(2) -ne $expected.Substring(2)) { throw 'operation_install_unverified' }
    $State.rows[$id] = $true
  }
}
function Assert-DocumentOperationSetup([hashtable]$State) {
  if (-not $State.layout -or -not $State.threadsDone -or -not $State.breakpointsDone -or $State.section -or $State.rows.Count -ne 14) { throw 'operation_install_unverified' }
}
function Get-DocumentOperationSetupState($State) {
  return @{ layoutVerified = ($null -ne $State -and $State.layout -eq $true)
    threadMatched = ($null -ne $State -and $State.threadsDone -eq $true)
    breakpointRows = $(if ($null -eq $State) { 0 } else { $State.rows.Count })
    installationVerified = ($null -ne $State -and $State.breakpointsDone -eq $true) }
}
