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
; /DChannel=beta builds "sigit beta": its own id, folder and shortcuts, so it installs beside
; the stable app instead of replacing it.
#ifdef Channel
  #if Channel == "beta"
    #define Beta
  #endif
#endif
#ifdef Beta
  #define AppTitle "sigit beta"
  #define AppSlug "sigit-beta"
#else
  #define AppTitle "sigit"
  #define AppSlug "sigit"
#endif

[Setup]
; Fixed for the life of the app: Windows uses it to recognise upgrades and the uninstaller.
#ifdef Beta
AppId={{4b9f6c2e-8d1a-4e57-9c3b-2a7e5f0d6b81}
#else
AppId={{76828bc8-c4d9-41d2-97df-d0ce35388fdd}
#endif
AppName={#AppTitle}
AppVersion={#AppVersion}
AppVerName={#AppTitle} {#AppVersion}
AppPublisher=sigit contributors
AppPublisherURL=https://github.com/moonmangit/sigit
AppSupportURL=https://github.com/moonmangit/sigit/issues
AppUpdatesURL=https://github.com/moonmangit/sigit/releases
DefaultDirName={autopf}\{#AppSlug}
DefaultGroupName={#AppTitle}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename={#AppSlug}-setup-windows-x64
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\sigit.exe
UninstallDisplayName={#AppTitle}
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
Name: "{autoprograms}\{#AppTitle}"; Filename: "{app}\sigit.exe"
Name: "{autodesktop}\{#AppTitle}"; Filename: "{app}\sigit.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\sigit.exe"; Description: "{cm:LaunchProgram,{#AppTitle}}"; Flags: nowait postinstall skipifsilent
