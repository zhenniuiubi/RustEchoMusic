@echo off
rem MSVC x64 build environment for this project.
rem Usage: build.bat [cargo args...]   e.g. build.bat build -p echo-app
set "MSVC_ROOT=C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
set "SDK_ROOT=C:\Program Files (x86)\Windows Kits\10"
set "PATH=%MSVC_ROOT%\bin\Hostx64\x64;%PATH%"
set "LIB=%MSVC_ROOT%\lib\x64;%SDK_ROOT%\Lib\10.0.26100.0\ucrt\x64;%SDK_ROOT%\Lib\10.0.26100.0\um\x64"
set "INCLUDE=%MSVC_ROOT%\include;%SDK_ROOT%\Include\10.0.26100.0\ucrt;%SDK_ROOT%\Include\10.0.26100.0\um;%SDK_ROOT%\Include\10.0.26100.0\shared;%SDK_ROOT%\Include\10.0.26100.0\winrt"
cargo %*
