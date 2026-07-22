@echo off
setlocal

REM ============================================================
REM  Syncthing Decrypt GUI - one-click build script
REM
REM  Usage: edit index.html / src/main.ts / src/styles.css,
REM         then double-click this script to rebuild.
REM  Output: dist\scdecrypt-gui-<version>-<arch>.exe
REM          (version read from the VERSION file)
REM
REM  NOTE: This file is intentionally ASCII-only. Non-ASCII
REM        characters in .bat files get corrupted by cmd.exe
REM        due to code-page (GBK/UTF-8) mismatches.
REM ============================================================

cd /d "%~dp0"

REM --- Add common toolchain locations to PATH (safe if absent) ---
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if exist "C:\Program Files\nodejs\node.exe"   set "PATH=C:\Program Files\nodejs;%PATH%"

REM --- Verify required tools ---
where node >nul 2>nul
if errorlevel 1 (
    echo [ERROR] Node.js not found in PATH.
    echo         Install it from https://nodejs.org/ then re-run this script.
    pause
    exit /b 1
)
where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] Rust / cargo not found in PATH.
    echo         Install it from https://rustup.rs/ then re-run this script.
    pause
    exit /b 1
)

REM --- Read version from VERSION file ---
if not exist "VERSION" (
    echo [ERROR] VERSION file not found in project root.
    echo         Create it with a single line like: 1.0.0
    pause
    exit /b 1
)
set /p VERSION=<VERSION

REM --- Detect target architecture from rustc host triple ---
set "ARCH=amd64"
for /f "tokens=2" %%a in ('rustc -vV 2^>nul ^| findstr /b "host:"') do (
    echo %%a | findstr /b "aarch64" >nul && set "ARCH=arm64"
    echo %%a | findstr /b "i686" >nul && set "ARCH=x86"
)
echo Version: %VERSION%   Architecture: %ARCH%

REM --- Proxy ----------------------------------------------------
REM This script does NOT set any proxy. It reuses whatever
REM HTTP_PROXY / HTTPS_PROXY already exist in your environment.
REM If dependency downloads fail behind a firewall, set them
REM in THIS console before running, for example:
REM     set HTTP_PROXY=http://your-host:port
REM     set HTTPS_PROXY=http://your-host:port
REM ---------------------------------------------------------------

REM --- Install frontend dependencies on first run ---
if not exist "node_modules" (
    echo [1/2] First run: installing frontend dependencies...
    call npm install
    if errorlevel 1 (
        echo.
        echo [ERROR] npm install failed.
        echo         Behind a firewall? Set HTTP_PROXY / HTTPS_PROXY and retry.
        pause
        exit /b 1
    )
) else (
    echo [1/2] Dependencies present, skipping npm install.
)

REM --- Build ---
echo [2/2] Building app - frontend bundle + Rust ...
call npx tauri build --no-bundle
if errorlevel 1 (
    echo.
    echo [ERROR] Build failed. See the messages above.
    pause
    exit /b 1
)

echo.
echo [OK] Build finished successfully.

REM --- Package output as scdecrypt-gui-<version>-<arch>.exe ---
if not exist "dist" mkdir dist
copy /y "src-tauri\target\release\scdecrypt-gui.exe" "dist\scdecrypt-gui-%VERSION%-%ARCH%.exe" >nul
echo      Output: %~dp0dist\scdecrypt-gui-%VERSION%-%ARCH%.exe
echo.
echo      Keep syncthing.exe beside the exe when running,
echo      otherwise the app will prompt you to download it.
echo.
pause
endlocal
