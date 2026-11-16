# Run from the matching GitHub Actions acceptance artifact. No SDK, system
# Python, account, administrator rights or production configuration required.
param([string]$OutDirectory = "")
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-PlainPath([string]$Root, [string]$Relative) {
  if ([IO.Path]::IsPathRooted($Relative) -or $Relative -match '(^|[\\/])\.\.([\\/]|$)' -or $Relative.Contains(':')) {
    throw "Unsafe package path"
  }
  $candidate = [IO.Path]::GetFullPath((Join-Path $Root $Relative))
  if (-not $candidate.StartsWith($Root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Package path leaves the artifact"
  }
  $current = $candidate
  while ($current.Length -ge $Root.Length) {
    $entry = Get-Item -LiteralPath $current -Force
    if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Package contains a reparse point" }
    if ($current -eq $Root) { break }
    $current = [IO.Path]::GetDirectoryName($current)
  }
  return $candidate
}

$bundle = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
$manifestPath = Assert-PlainPath $bundle "acceptance-manifest.json"
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
if ($manifest.schemaVersion -ne 1 -or $manifest.commit -notmatch '^[a-f0-9]{40}$') { throw "Invalid package manifest" }
$seen = @{}
foreach ($file in $manifest.files) {
  if ($seen.ContainsKey($file.path)) { throw "Duplicate package path" }
  $seen[$file.path] = $true
  $candidate = Assert-PlainPath $bundle $file.path
  if ((Get-Item -LiteralPath $candidate).PSIsContainer -or $file.sha256 -notmatch '^[a-f0-9]{64}$') { throw "Invalid package file" }
  if ((Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant() -ne $file.sha256) { throw "Package checksum mismatch" }
}
if (-not $seen.ContainsKey('verify_document_engine.exe') -or -not $seen.ContainsKey('runtime/manifest.json')) { throw "Incomplete acceptance package" }
# Extra files must not inject a runtime module that was not in the manifest.
foreach ($file in Get-ChildItem -LiteralPath $bundle -Recurse -File -Force) {
  $relative = $file.FullName.Substring($bundle.Length + 1).Replace('\', '/')
  if ($relative -ne 'acceptance-manifest.json' -and -not $seen.ContainsKey($relative)) { throw "Unlisted package file" }
}
if (-not $OutDirectory) { $OutDirectory = Join-Path ([IO.Path]::GetTempPath()) ("OfoxDocumentCheck-" + [Guid]::NewGuid().ToString('N')) }
$OutDirectory = [IO.Path]::GetFullPath($OutDirectory)
if (Test-Path -LiteralPath $OutDirectory) { throw "Use a new output directory to preserve earlier evidence" }
New-Item -ItemType Directory -Path $OutDirectory | Out-Null
$report = Join-Path $OutDirectory 'probe.json'
& (Join-Path $bundle 'verify_document_engine.exe') --runtime (Join-Path $bundle 'runtime') --out $report
$probeExit = $LASTEXITCODE
$summary = [ordered]@{ schemaVersion = 1; commit = $manifest.commit; probeExit = $probeExit; uiAcceptance = 'pending'; report = 'probe.json' }
$summary | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $OutDirectory 'acceptance.json')
Write-Host "Acceptance evidence: $OutDirectory"
if ($probeExit -ne 0) { throw "Native document engine checks failed; Python parity remains unaccepted" }
Write-Host 'Native probes passed. Complete the client UI checklist before claiming installation acceptance.'
