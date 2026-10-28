; Instalador de xdir (Inno Setup 6).
; Uso: ISCC.exe /DAppVersion=0.1.6 /DSourceExe=..\target\release\xdir.exe /DOutputDir=..\dist installer\xdir.iss
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
; Esquema clásico: el exe en Program Files para todos los usuarios (pide admin) y la configuración
; de cada usuario en %APPDATA%\xdir, que la crea la app y la desinstalación borra (ver [UninstallDelete]).
PrivilegesRequired=admin
; Borrar {userappdata} en una instalación con admin es intencional.
UsedUserAreasWarning=no
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
; Nombre en "Aplicaciones instaladas" (sin la versión, que va en su propia columna).
UninstallDisplayName=xdir
SetupIconFile=..\assets\xdir.ico
CloseApplications=yes

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\xdir"; Filename: "{app}\xdir.exe"
Name: "{autodesktop}\xdir"; Filename: "{app}\xdir.exe"; Tasks: desktopicon

[UninstallDelete]
; Inno solo borra las carpetas que creó; esto cubre una carpeta que ya existía al instalar.
Type: dirifempty; Name: "{app}"
; Desinstalar no deja nada: también la sesión y los favoritos del usuario que desinstala.
Type: filesandordirs; Name: "{userappdata}\xdir"

[Run]
Filename: "{app}\xdir.exe"; Description: "{cm:LaunchProgram,xdir}"; Flags: nowait postinstall skipifsilent runasoriginaluser
; Actualización desde la propia app (setup /SILENT /RELAUNCH): vuelve a abrir xdir al terminar.
Filename: "{app}\xdir.exe"; Flags: nowait runasoriginaluser; Check: Relaunch

[Code]
const
  WM_CLOSE = $0010;

// ¿Se pasó /RELAUNCH? Lo usa la actualización automática de la app para que xdir se vuelva a abrir.
function Relaunch(): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/RELAUNCH') = 0 then
      Result := True;
end;

// Cierra las ventanas de xdir abiertas (clase "xdir.main", ver CLASS_NAME en src/main.rs) para que
// el desinstalador pueda borrar el exe. WM_CLOSE deja que la app guarde la sesión; si a los 5 s
// sigue abierta, se la termina.
procedure CloseXdir();
var
  Tries, Code: Integer;
begin
  Tries := 0;
  while (FindWindowByClassName('xdir.main') <> 0) and (Tries < 50) do begin
    PostMessage(FindWindowByClassName('xdir.main'), WM_CLOSE, 0, 0);
    Sleep(100);
    Tries := Tries + 1;
  end;
  if FindWindowByClassName('xdir.main') <> 0 then
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM xdir.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function InitializeUninstall(): Boolean;
begin
  CloseXdir();
  Result := True;
end;
