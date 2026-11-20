param([switch]$ShortFlags, [switch]$UnreadableFlags)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/document-engine-stdio.ps1')
$values = @(Get-DocumentCdbArguments 42 'private symbols' 'private commands')
# Exercise the production line reader without executing the Windows controller.
$tokens = $null; $parse = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
  (Join-Path $PSScriptRoot '../../../scripts/document-engine-cdb.ps1'), [ref]$tokens, [ref]$parse)
if ($parse.Count -ne 0) { throw 'controller_parse_failed' }
$function = $ast.Find({ param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Read-CdbLine'
}, $false)
. ([ScriptBlock]::Create($function.Extent.Text))
$script:writer = [IO.StringWriter]::new()
$script:capturedBytes = 0; $script:stdoutLines = 0; $script:stderrLines = 0
$script:promptLines = 0; $script:markerMentionLines = 0; $script:numericTargetLines = 0
$script:gflagValueLines = 0; $script:nullCharacters = 0
$script:reportedWin32Errors = @()
$script:setupStep = 0; $script:targetSeen = $false; $script:before = $null; $script:after = $null
$script:section = ''; $script:tail = $false
$script:gflagHints = @{ symbols = $false; types = $false; peb = $false; globalFlags = $false; failure = $false; extension = $false }
try {
  Read-CdbLine '0:000> .printf "OFOX_CDB_RAW_V1 TARGET %x %x END", @$tpid, @$tid' $false
  Read-CdbLine '0:000> OFOX_CDB_RAW_V1 TARGET 2a 2b END' $false
  Read-CdbLine ("private-token" + [char]0) $false
  Read-CdbLine 'Cannot attach to process 42, Win32 error 0n5. private-token' $false
  Read-CdbLine 'debugger failure, Win32 error 0x00000057' $false
  Read-CdbLine 'unknown Win32 error 0n4294967296' $false
  Read-CdbLine 'ERROR symbol type PEB NtGlobalFlag extension private-token' $false
  $shape = @{ promptLines = $promptLines; markerMentionLines = $markerMentionLines
    numericTargetLines = $numericTargetLines; nullCharacters = $nullCharacters; targetSeen = $targetSeen
    reportedWin32Errors = @($reportedWin32Errors) }
  $beforeText = if ($ShortFlags) { '0' } else { '00000070' }
  $afterText = if ($ShortFlags) { '2' } else { '00000072' }
  Read-CdbLine 'OFOX_CDB_RAW_V1 TARGET 2a 2b END' $false
  Read-CdbLine 'OFOX_CDB_RAW_V1 BEFORE_BEGIN' $false
  if ($UnreadableFlags) {
    Read-CdbLine 'GetExpression: not found: ntdll!NtGlobalFlag; GetFieldValue type PEB extension private-token' $false
    foreach ($bad in @('0x100000000', '-1', '0x2 private-token')) {
      Read-CdbLine "Current NtGlobalFlag contents: $bad" $false
    }
    $failureStage = $null
    try { Read-CdbLine 'OFOX_CDB_RAW_V1 BEFORE_END' $false } catch { $failureStage = $_.Exception.Message }
    [Console]::Out.WriteLine((@{ hints = $gflagHints; failureStage = $failureStage; before = $before; tail = $tail } | ConvertTo-Json -Compress))
    exit
  }
  foreach ($line in @(
    "Current NtGlobalFlag contents: 0x$beforeText", 'OFOX_CDB_RAW_V1 BEFORE_END',
    "New NtGlobalFlag contents: 0x$afterText", 'OFOX_CDB_RAW_V1 AFTER_BEGIN',
    "Current NtGlobalFlag contents: 0x$afterText", 'OFOX_CDB_RAW_V1 AFTER_END', 'OFOX_CDB_RAW_V1 CONFIG_END'
  )) { Read-CdbLine $line $false }
  [Console]::Out.WriteLine((@{ arguments = $values; shape = $shape
    configured = @{ targetSeen = $targetSeen; pid = $targetPid; tid = $targetTid
      before = $before; after = $after; tail = $tail; gflagValueLines = $gflagValueLines }; hints = $gflagHints
  } | ConvertTo-Json -Depth 4 -Compress))
} finally { $writer.Dispose() }
