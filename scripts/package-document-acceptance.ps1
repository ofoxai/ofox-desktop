# CI-only packaging; all inputs are produced by this exact checkout.
param([Parameter(Mandatory=$true)][string]$Commit, [Parameter(Mandatory=$true)][string]$OutDirectory)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
if ($Commit -notmatch '^[a-f0-9]{40}$' -or (git rev-parse HEAD).Trim() -ne $Commit) { throw "Checkout identity mismatch" }
if (git status --porcelain --untracked-files=no) { throw "Tracked checkout is dirty" }
if (Test-Path -LiteralPath $OutDirectory) { throw "Never overwrite an acceptance package" }
New-Item -ItemType Directory -Path $OutDirectory | Out-Null
Copy-Item -LiteralPath 'src-tauri/target/release/verify_document_engine.exe' -Destination $OutDirectory
Copy-Item -LiteralPath 'assistant-runtime/resources' -Destination (Join-Path $OutDirectory 'runtime') -Recurse
Copy-Item -LiteralPath 'scripts/test-document-engine.ps1' -Destination $OutDirectory
Copy-Item -LiteralPath 'docs/document-engine-client-acceptance.md' -Destination $OutDirectory
$base = [IO.Path]::GetFullPath($OutDirectory).TrimEnd('\')
$files = @(Get-ChildItem -LiteralPath $base -Recurse -File -Force | Sort-Object FullName | ForEach-Object {
  if (($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Cannot package reparse points" }
  [ordered]@{ path = $_.FullName.Substring($base.Length + 1).Replace('\', '/'); sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
})
[ordered]@{ schemaVersion = 1; commit = $Commit; files = $files } | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $base 'acceptance-manifest.json')
