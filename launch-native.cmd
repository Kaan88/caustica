@echo off
setlocal
pushd "%~dp0native"
if not exist "target\release\cornell-rtx.exe" (
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup.ps1
  if errorlevel 1 (
    pause
    popd
    exit /b 1
  )
)
start "" "target\release\cornell-rtx.exe" %*
popd
