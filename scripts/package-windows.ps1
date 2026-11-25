# Build a portable .zip into dist/.  usage: package-windows.ps1 -Target <rust target> -Arch <name>
param(
    [Parameter(Mandatory)] [string] $Target,
    [Parameter(Mandatory)] [string] $Arch
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build --release --locked --target $Target
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"').Matches[0].Groups[1].Value
$name = "FluxDM-$version-windows-$Arch"
$work = "target/package/windows/$name"
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force "$work/extension", dist | Out-Null
Copy-Item "target/$Target/release/fluxdm.exe", "target/$Target/release/fluxdm-host.exe" $work
foreach ($f in 'manifest.json','background.js','content.js','popup.js','links.js','rules.js','popup.html','links.html','theme.css','icons') {
    Copy-Item -Recurse "extension/$f" "$work/extension/"
}
Copy-Item crates/app/assets/LICENSE-*.txt, packaging/README-windows.txt, packaging/install-browser-helper.cmd $work
Compress-Archive -Path $work -DestinationPath "dist/$name.zip" -Force
Write-Output "dist/$name.zip"
