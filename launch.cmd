@echo off
setlocal
pushd "%~dp0"
if not exist "target\release\caustica.exe" (
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup.ps1
  if errorlevel 1 (
    pause
    popd
    exit /b 1
  )
)
start "" "target\release\caustica.exe" %*
popd
