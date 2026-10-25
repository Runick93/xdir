; Instalador de xdir (Inno Setup 6).
; Uso: ISCC.exe /DAppVersion=0.1.0 /DSourceExe=..\target\release\xdir.exe /DOutputDir=..\dist installer\xdir.iss
; Lo invoca scripts\package.ps1; las rutas relativas son respecto de este archivo.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\xdir.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{6F2B8E4A-3C1D-4E7F-9A5B-2D8C0E1F7A34}
AppName=xdir
AppVersion={#AppVersion}
AppVerName=xdir {#AppVersion}
AppPublisher=Runick
DefaultDirName={autopf}\xdir
DefaultGroupName=xdir
DisableProgramGroupPage=yes
; Sin admin instala para el usuario actual; con admin se puede elegir para todos.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1809.
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename=xdir-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\xdir.exe
SetupIconFile=..\assets\xdir.ico
CloseApplications=yes

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\xdir"; Filename: "{app}\xdir.exe"
Name: "{autodesktop}\xdir"; Filename: "{app}\xdir.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\xdir.exe"; Description: "{cm:LaunchProgram,xdir}"; Flags: nowait postinstall skipifsilent
