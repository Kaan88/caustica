; Caustica installer (Inno Setup 6). Built only by GitHub Actions (.github/workflows/ci.yml):
;   iscc /DAppVersion=0.1.4 /DSourceDir=<the packaged folder> installer\caustica.iss
; Local builds don't need it: run target\release\caustica.exe or launch.cmd.
;
; The app never writes beside itself when installed (src/dirs.rs): settings, prefs, user scenes,
; downloaded assets and captures go to %APPDATA%\Caustica, thumbnails and catalogs to
; %LOCALAPPDATA%\Caustica. So it installs per user (no admin prompt) by default, or for all users
; into Program Files from the setup's dialog. A `portable` file beside the exe keeps everything
; beside it instead (for unzipped copies, never written by this installer).
;
; SourceDir's scenes\ includes the models, HDRIs and textures the shipped scenes use (~870 MB,
; fetched by tools/fetch_assets.ps1 in CI); without them those scenes can't load when installed,
; since the app finds relative asset paths only in the user folder and {app}\scenes.

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
PrivilegesRequiredOverridesAllowed=dialog
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
; The bundled assets make the solid block large: compress it on several threads.
LZMANumBlockThreads=4
; An update installs over the previous version (same AppId); close a running copy first.
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Caustica"; Filename: "{app}\caustica.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Caustica"; Filename: "{app}\caustica.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[UninstallDelete]
; Caches only (thumbnails, asset catalogs): rebuilt on demand.
Type: filesandordirs; Name: "{localappdata}\Caustica"

[Run]
Filename: "{app}\caustica.exe"; WorkingDir: "{app}"; Description: "{cm:LaunchProgram,Caustica}"; Flags: nowait postinstall skipifsilent
; A silent update from the app (update.rs: /relaunch=1) starts the new version itself.
Filename: "{app}\caustica.exe"; WorkingDir: "{app}"; Flags: nowait runasoriginaluser; Check: IsUpdate

[Code]
// /relaunch=1 on the setup's command line, as the app's self-update passes it.
function IsUpdate: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;

// The per-user folder holds the user's own scenes, downloads and captures: ask before removing.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Dir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Dir := ExpandConstant('{userappdata}\Caustica');
    if DirExists(Dir) and not UninstallSilent and
       (MsgBox('Also delete your Caustica settings, saved scenes, downloaded assets and captures in ' + Dir + '?',
               mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES) then
      DelTree(Dir, True, True, True);
  end;
end;
