# Builds a reproducible, offline-friendly Windows release folder with checksums.
# Code signing is deliberately not required (it would cost money); verify with the SHA-256 list.
param([string]$OutDir = "dist")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$env:SOURCE_DATE_EPOCH = (git log -1 --format=%ct)
cargo build --release --locked --bin openclaim --bin openclaimd

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches.Groups[1].Value
$stage = Join-Path $OutDir "openclaim-$version-windows-x64"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item target/release/openclaim.exe, target/release/openclaimd.exe $stage
Copy-Item README.md, LICENSE $stage
Copy-Item -Recurse docs (Join-Path $stage "docs")
Copy-Item -Recurse source-manifests (Join-Path $stage "source-manifests")
if (Test-Path apps/web/dist/web/browser) { Copy-Item -Recurse apps/web/dist/web/browser (Join-Path $stage "gui") }
Copy-Item -Recurse apps/browser-extension (Join-Path $stage "browser-extension") -Exclude node_modules

& "$stage/openclaim.exe" completion powershell | Out-File -Encoding utf8 (Join-Path $stage "openclaim-completion.ps1")
& "$stage/openclaim.exe" help-tree --output json | Out-File -Encoding utf8 (Join-Path $stage "docs/help-tree.json")

cargo tree --workspace --edges normal --format "{p} {l}" | Sort-Object -Unique | Out-File -Encoding utf8 (Join-Path $stage "THIRD-PARTY-LICENSES.txt")

$zip = "$stage.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path "$stage/*" -DestinationPath $zip
Get-FileHash -Algorithm SHA256 $zip, "$stage/openclaim.exe", "$stage/openclaimd.exe" |
  ForEach-Object { "$($_.Hash.ToLower())  $(Split-Path -Leaf $_.Path)" } |
  Out-File -Encoding ascii (Join-Path $OutDir "SHA256SUMS.txt")
Write-Host "release at $zip"
