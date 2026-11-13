param(
    [string]$Makensis,
    [string]$TargetDir = "target/release-windows",
    [string]$OutputDir = "dist"
)
$ErrorActionPreference = "Stop"
Push-Location (Join-Path $PSScriptRoot "..")
try {
    if (-not $Makensis) {
        $command = Get-Command makensis -ErrorAction SilentlyContinue
        $Makensis = if ($command) { $command.Source } else { Join-Path ${env:ProgramFiles(x86)} "NSIS/makensis.exe" }
    }
    if (-not (Test-Path -LiteralPath $Makensis)) {
        throw "NSIS was not found. Install NSIS or pass -Makensis with the path to makensis.exe."
    }
    $Makensis = (Resolve-Path -LiteralPath $Makensis).Path
    $version = (cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json).packages |
        Where-Object name -eq "app" | Select-Object -ExpandProperty version
    if ($LASTEXITCODE -ne 0 -or -not $version) { throw "Could not read the Airframe version." }
    cargo build --locked --release -p app --target x86_64-pc-windows-msvc --target-dir $TargetDir
    if ($LASTEXITCODE -ne 0) { throw "Airframe build failed." }
    $app = (Resolve-Path -LiteralPath (Join-Path $TargetDir "x86_64-pc-windows-msvc/release/airframe.exe")).Path
    $output = (New-Item -ItemType Directory -Path $OutputDir -Force).FullName
    $installer = Join-Path $output "Airframe-Setup-x64.exe"
    & $Makensis "/DAPP_EXE=$app" "/DSETUP_EXE=$installer" "/DVERSION=$version" (Join-Path $PSScriptRoot "Airframe.nsi")
    if ($LASTEXITCODE -ne 0) { throw "NSIS build failed." }
    Get-FileHash -Algorithm SHA256 -LiteralPath $installer
} finally {
    Pop-Location
}
