@echo off
rem Build + package EchoMusicLite for release, with optional UPX compression.
rem
rem The binary is already built with opt-level=z (release ~8.2M).
rem UPX shrinks it a further ~50% (8.2M -> ~4M), well under the 5M target.
rem Install upx once (one-time), it must be on PATH when you run this script:
rem   scoop install upx      (recommended)
rem   choco install upx
rem   or download from https://github.com/upx/upx/releases
rem
rem If upx is not on PATH, the script still leaves an uncompressed ~8.2M binary.
rem Re-run after installing upx to reach ~4M.
cd /d "%~dp0.." || exit /b 1

call build.bat build --release -p echo-app --offline
if errorlevel 1 exit /b 1

copy /Y target\release\EchoMusicLite.exe dist\EchoMusicLite.exe

where upx >nul 2>nul
if %errorlevel%==0 (
  echo [pack] upx found - compressing dist\EchoMusicLite.exe ...
  upx --best --lzma dist\EchoMusicLite.exe
) else (
  echo [pack] upx NOT found - binary left uncompressed (~10M).
  echo        Install upx ^(scoop install upx^) and re-run this script to reach ~5M.
)
