; EAW:Remake setup (Inno Setup 6). Built by tools\package.ps1; by hand:
;   ISCC /DAppVersion=0.1.0 /DExePath=<release exe built with --features dist> /O<output folder> tools\installer\eaw-remake.iss
;
; Installs for the current user only (no admin rights, no UAC prompt). The setup holds only our own files: the game exe,
; LICENSE, CREDITS.md and a readme. The game and the Remake mod come from the player's Steam and are never copied.
; The uninstaller removes what the setup installed and keeps %APPDATA%\SpacePeace (saves, settings, logs).

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef ExePath
  #define ExePath "..\..\target\dist\release\eaw-remake.exe"
#endif

#define AppName "EAW:Remake"
#define AppExe "eaw-remake.exe"
#define GameStoreUrl "https://store.steampowered.com/app/32470/"
#define ModWorkshopUrl "https://steamcommunity.com/sharedfiles/filedetails/?id=1770851727"

[Setup]
AppId={{4C94FBD2-EB8E-489C-B020-DCB50D6AB396}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=msch128
AppPublisherURL=https://github.com/msch128/eaw-remake
AppSupportURL=https://github.com/msch128/eaw-remake/issues
AppUpdatesURL=https://github.com/msch128/eaw-remake/releases
; The name has a colon: not usable in folder and shortcut names.
DefaultDirName={autopf}\EAW-Remake
DefaultGroupName=EAW Remake
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#AppExe}
; A running game is closed for an update but never started again behind the player's back.
RestartApplications=no
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
OutputBaseFilename=EAW-Remake-Setup-{#AppVersion}
VersionInfoVersion={#AppVersion}.0
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
WelcomeLabel2=This will install [name/ver] on your computer.%n%nEAW:Remake needs your own copy of Star Wars: Empire at War - Gold Pack and the Empire at War Remake 4.0 mod, both from Steam. They are not part of this setup. When it is done it tells you if one of them is still missing.%n%nIt is recommended that you close all other applications before continuing.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\CREDITS.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\EAW Remake"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"
Name: "{group}\Uninstall EAW Remake"; Filename: "{uninstallexe}"
Name: "{autodesktop}\EAW Remake"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\README.txt"; Description: "View the readme"; Flags: shellexec postinstall skipifsilent unchecked

[Code]
{ Opens a web page in the player's browser. }
procedure OpenPage(const Url: String);
var
  ErrorCode: Integer;
begin
  ShellExec('open', Url, '', '', SW_SHOWNORMAL, ewNoWait, ErrorCode);
end;

{ Asks the installed game whether the game and the Remake mod are there (see crates/space-peace/src/install_check.rs):
  exit code 0 both found, 2 the game is missing, 3 the mod is missing. The install is finished either way, so the player
  can get the missing part from Steam afterwards. Silent setups only write the answer to the log (/LOG). }
procedure CheckInstall;
var
  ExitCode: Integer;
begin
  if not Exec(ExpandConstant('{app}\{#AppExe}'), '--check-install', ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) then
  begin
    Log('--check-install could not be started: ' + SysErrorMessage(ExitCode));
    Exit;
  end;
  Log(Format('--check-install exit code %d', [ExitCode]));
  if WizardSilent then
    Exit;
  case ExitCode of
    2:
      if SuppressibleMsgBox(
           'EAW:Remake is installed, but Star Wars: Empire at War - Gold Pack was not found on this PC.' + #13#10#13#10 +
           'EAW:Remake needs your own copy of the game from Steam (app 32470, it includes Forces of Corruption). ' +
           'Install it, then subscribe to the Empire at War Remake 4.0 mod on the Steam Workshop (id 1770851727) and start EAW:Remake again.' + #13#10#13#10 +
           'Open the Steam store page of the game now?',
           mbInformation, MB_YESNO, IDNO) = IDYES then
        OpenPage('{#GameStoreUrl}');
    3:
      if SuppressibleMsgBox(
           'EAW:Remake is installed and found Star Wars: Empire at War, but the Empire at War Remake 4.0 mod is missing.' + #13#10#13#10 +
           'Subscribe to the mod on the Steam Workshop (id 1770851727), let Steam finish the download and start EAW:Remake again.' + #13#10#13#10 +
           'Open the mod page on the Steam Workshop now?',
           mbInformation, MB_YESNO, IDNO) = IDYES then
        OpenPage('{#ModWorkshopUrl}');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    CheckInstall;
end;
