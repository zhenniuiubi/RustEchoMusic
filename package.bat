@echo off
rem EchoMusic packaging script: release build -> dist\EchoMusic.exe
rem Usage: package.bat
rem
rem NOTE: keep every comment here ASCII-only. cmd.exe reads a UTF-8 file as
rem GBK, so CJK comment bytes can swallow the following line -- that is how
rem CARGO_BUILD_JOBS=2 silently got lost before.
rem
rem NOTE: full-parallel release builds hang on this machine (rustc stalls,
rem CPU drops to zero), so CARGO_BUILD_JOBS is pinned to 2.
rem First build ~6-8 min, incremental under 1 min.
setlocal
cd /d "%~dp0"

set "CARGO_BUILD_JOBS=2"
call "%~dp0build.bat" build --release -p echo-app
if errorlevel 1 (
  echo [ERROR] release build failed
  exit /b 1
)

if not exist "%~dp0dist" mkdir "%~dp0dist"
copy /y "%~dp0target\release\echo-music.exe" "%~dp0dist\EchoMusic.exe" >nul
if errorlevel 1 (
  echo [ERROR] copy failed
  exit /b 1
)

echo.
echo [OK] dist\EchoMusic.exe
for %%F in ("%~dp0dist\EchoMusic.exe") do echo      size: %%~zF bytes
endlocal
