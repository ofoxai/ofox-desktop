param([string]$Directory)
Start-Sleep -Milliseconds 100
[IO.File]::WriteAllText((Join-Path $Directory 'go.tmp'), '{"hostResumeResult":1}')
[IO.File]::Move((Join-Path $Directory 'go.tmp'), (Join-Path $Directory 'go.json'))
