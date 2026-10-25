@echo off
rem Compila usando el entorno de Visual Studio Build Tools (tiene las librerias de escritorio x64).
rem Uso: scripts\build.cmd [argumentos de cargo]   ej: scripts\build.cmd run --release
setlocal
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "TMPF=%TEMP%\xdir_vs_path.txt"
"%VSWHERE%" -products Microsoft.VisualStudio.Product.BuildTools -latest -property installationPath > "%TMPF%"
set "XDIR_VS="
set /p XDIR_VS=<"%TMPF%"
del "%TMPF%" 2>nul
if not defined XDIR_VS goto :novs
rem vcvars64 invoca vswhere por su nombre, asi que tiene que estar en el PATH.
set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
call "%XDIR_VS%\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if "%~1"=="" (cargo build --release) else (cargo %*)
exit /b %errorlevel%

:novs
echo No se encontro Visual Studio Build Tools.
exit /b 1
