# Empaqueta un xdir.exe ya compilado: zip portable e instalador (Inno Setup).
# Uso: pwsh scripts\package.ps1 [-Exe target\release\xdir.exe] [-OutDir dist] [-NoInstaller]
# Deja en OutDir: xdir-<version>-portable.zip y xdir-<version>-setup.exe.
param(
    [string]$Exe = "target\release\xdir.exe",
    [string]$OutDir = "dist",
    [switch]$NoInstaller
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path $Exe)) { throw "No existe $Exe; compilar antes con scripts\build.cmd" }
$Exe = (Resolve-Path $Exe).Path

# Versión: la de [package] en Cargo.toml.
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "No se encontró la versión en Cargo.toml" }

New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

# Zip portable: el exe más xdir.portable, que hace que la sesión se guarde junto al exe.
$stage = Join-Path $OutDir "xdir-$version-portable"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item $Exe $stage
New-Item -ItemType File (Join-Path $stage "xdir.portable") | Out-Null
$zip = "$stage.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item -Recurse -Force $stage
Write-Host "Zip:        $zip"

if ($NoInstaller) { return }

# Instalador: ISCC del PATH o de las ubicaciones habituales de Inno Setup 6.
$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw "No se encontró Inno Setup 6 (ISCC.exe). Instalar o usar -NoInstaller." }

& $iscc /Q "/DAppVersion=$version" "/DSourceExe=$Exe" "/DOutputDir=$OutDir" installer\xdir.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC falló ($LASTEXITCODE)" }
Write-Host "Instalador: $(Join-Path $OutDir "xdir-$version-setup.exe")"
