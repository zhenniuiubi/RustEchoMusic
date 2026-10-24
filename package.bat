@echo off
rem EchoMusic 打包脚本：release 构建并输出到 dist\EchoMusic.exe
rem 用法：package.bat
rem
rem 注意：全并行 release 编译在本机会卡死（rustc 挂起、CPU 停走），
rem 因此固定 CARGO_BUILD_JOBS=2；首次约 6~8 分钟，增量编译 1 分钟内。
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
