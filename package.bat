@echo off
setlocal

echo ============================================
echo  Vectorize — Release Build + Package
echo ============================================
echo.

:: Build release binary (optimized, ~8-10x faster than debug)
echo [1/3] Building release binary...
cargo build -p vectorize-gui --release
if errorlevel 1 (
    echo BUILD FAILED
    pause
    exit /b 1
)
echo       OK

:: Create dist folder
echo [2/3] Packaging...
set DIST=dist\Vectorize
if exist dist rmdir /s /q dist
mkdir "%DIST%"

:: Copy the binary
copy target\release\vectorize-gui.exe "%DIST%\Vectorize.exe" >nul

:: Copy optional config (app works without it via hardcoded defaults)
copy crates\vectorize-gui\ui_config.json "%DIST%\ui_config.json" >nul 2>nul

:: Copy icon asset
if exist crates\vectorize-gui\assets\logo.png (
    mkdir "%DIST%\assets" 2>nul
    copy crates\vectorize-gui\assets\logo.png "%DIST%\assets\logo.png" >nul
)

echo       OK

:: Show result
echo [3/3] Done!
echo.
echo   Output: dist\Vectorize\
echo.
dir /b "%DIST%"
echo.
for %%A in ("%DIST%\Vectorize.exe") do echo   Vectorize.exe: %%~zA bytes
echo.
echo  To distribute: zip the dist\Vectorize folder.
echo  Users just unzip and double-click Vectorize.exe.
echo.
pause
