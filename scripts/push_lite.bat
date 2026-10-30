@echo off
rem Push the Lite branch to origin.
rem
rem Usage (run from anywhere):
rem   scripts\push_lite.bat            -> uses proxy port 7890
rem   scripts\push_lite.bat 7890       -> explicit proxy port
rem
rem The default 7890 is the local Clash / proxy port. Change the default
rem below (or pass a port) if your proxy listens elsewhere.
rem schannelCheckRevoke=false avoids CRYPT_E_REVOCATION_OFFLINE on git-for-windows.

set "PORT=%1"
if "%PORT%"=="" set "PORT=7890"

cd /d "%~dp0.." || exit /b 1

git -c http.proxy=http://127.0.0.1:%PORT% ^
    -c https.proxy=http://127.0.0.1:%PORT% ^
    -c http.schannelCheckRevoke=false ^
    push origin Lite
