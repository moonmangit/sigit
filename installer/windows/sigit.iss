; Windows installer for sigit (Inno Setup 6). Built by .github/workflows/release.yml:
;   iscc /DAppVersion=0.1.1 /DSourceExe=path\to\sigit.exe /O<out dir> installer\windows\sigit.iss
; Installs for the current user (no admin prompt) into %LOCALAPPDATA%\Programs\sigit, with a
; Start menu entry, an optional desktop shortcut and an uninstaller. Settings in %APPDATA%\sigit
; are left alone by uninstall, so reinstalling keeps them.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\sigit.exe"
#endif

[Setup]
; Fixed for the life of the app: Windows uses it to recognise upgrades and the uninstaller.
AppId={{76828bc8-c4d9-41d2-97df-d0ce35388fdd}
AppName=sigit
AppVersion={#AppVersion}
AppVerName=sigit {#AppVersion}
AppPublisher=sigit contributors
AppPublisherURL=https://github.com/moonmangit/sigit
AppSupportURL=https://github.com/moonmangit/sigit/issues
AppUpdatesURL=https://github.com/moonmangit/sigit/releases
DefaultDirName={autopf}\sigit
DefaultGroupName=sigit
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=sigit-setup-windows-x64
LicenseFile=..\..\LICENSE
UninstallDisplayIcon={app}\sigit.exe
UninstallDisplayName=sigit
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Upgrading over a running copy: ask to close it first.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "sigit.exe"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\sigit"; Filename: "{app}\sigit.exe"
Name: "{autodesktop}\sigit"; Filename: "{app}\sigit.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\sigit.exe"; Description: "{cm:LaunchProgram,sigit}"; Flags: nowait postinstall skipifsilent
