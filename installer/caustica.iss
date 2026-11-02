; Caustica installer (Inno Setup 6). Built only by GitHub Actions (.github/workflows/ci.yml):
;   iscc /DAppVersion=0.1.4 /DSourceDir=<the packaged folder> installer\caustica.iss
; Local builds don't need it: run target\release\caustica.exe or launch.cmd.
;
; Per-user install (no admin prompt): the app writes settings, captures, thumbnails and
; downloaded assets beside itself, which Program Files would not allow.

#ifndef AppVersion
  #define AppVersion "0.0.0-local"
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<folder with caustica.exe, its DLLs and scenes>
#endif

[Setup]
AppId={{6400E487-F9B6-484E-B957-B4D0B9A5D890}
AppName=Caustica
AppVersion={#AppVersion}
AppVerName=Caustica {#AppVersion}
AppPublisher=Kaan
VersionInfoProductTextVersion={#AppVersion}
VersionInfoDescription=Caustica setup
DefaultDirName={autopf}\Caustica
DefaultGroupName=Caustica
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern
SetupIconFile=..\assets\caustica.ico
UninstallDisplayIcon={app}\caustica.exe
UninstallDisplayName=Caustica
OutputDir=..\dist
OutputBaseFilename=caustica-v{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
; An update installs over the previous version (same AppId); close a running copy first.
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Caustica"; Filename: "{app}\caustica.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Caustica"; Filename: "{app}\caustica.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\caustica.exe"; WorkingDir: "{app}"; Description: "{cm:LaunchProgram,Caustica}"; Flags: nowait postinstall skipifsilent
