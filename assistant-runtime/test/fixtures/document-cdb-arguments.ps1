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
try {
  Read-CdbLine '0:000> .printf "OFOX_CDB_RAW_V1 TARGET %x %x END", @$tpid, @$tid' $false
  Read-CdbLine '0:000> OFOX_CDB_RAW_V1 TARGET 2a 2b END' $false
  Read-CdbLine ("private-token" + [char]0) $false
  Read-CdbLine 'Cannot attach to process 42, Win32 error 0n5. private-token' $false
  Read-CdbLine 'debugger failure, Win32 error 0x00000057' $false
  Read-CdbLine 'unknown Win32 error 0n4294967296' $false
  $shape = @{ promptLines = $promptLines; markerMentionLines = $markerMentionLines
    numericTargetLines = $numericTargetLines; nullCharacters = $nullCharacters; targetSeen = $targetSeen
    reportedWin32Errors = @($reportedWin32Errors) }
  foreach ($line in @(
    'OFOX_CDB_RAW_V1 TARGET 2a 2b END', 'OFOX_CDB_RAW_V1 BEFORE_BEGIN',
    'Current NtGlobalFlag contents: 0x00000070', 'OFOX_CDB_RAW_V1 BEFORE_END',
    'New NtGlobalFlag contents: 0x00000072', 'OFOX_CDB_RAW_V1 AFTER_BEGIN',
    'Current NtGlobalFlag contents: 0x00000072', 'OFOX_CDB_RAW_V1 AFTER_END', 'OFOX_CDB_RAW_V1 CONFIG_END'
  )) { Read-CdbLine $line $false }
  [Console]::Out.WriteLine((@{ arguments = $values; shape = $shape
    configured = @{ targetSeen = $targetSeen; pid = $targetPid; tid = $targetTid
      before = $before; after = $after; tail = $tail; gflagValueLines = $gflagValueLines }
  } | ConvertTo-Json -Depth 4 -Compress))
} finally { $writer.Dispose() }
