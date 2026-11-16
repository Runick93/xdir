; Instalador de xdir (Inno Setup 6).
; Uso: ISCC.exe /DAppVersion=0.1.6 /DSourceExe=..\target\release\xdir-windows.exe /DOutputDir=..\dist installer\xdir.iss
; Lo invoca scripts\package.ps1; las rutas relativas son respecto de este archivo.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\xdir-windows.exe"
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
; Imágenes del asistente (una por escala de pantalla; las genera scripts\make-installer-images.py).
; Inno 6 oculta por defecto la página de bienvenida, que es donde va el panel lateral.
DisableWelcomePage=no
WizardImageFile=wizard-164.bmp,wizard-192.bmp,wizard-246.bmp,wizard-273.bmp,wizard-328.bmp,wizard-355.bmp
WizardSmallImageFile=wizard-small-55.bmp,wizard-small-64.bmp,wizard-small-73.bmp,wizard-small-82.bmp,wizard-small-92.bmp,wizard-small-110.bmp
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
; El exe compilado se llama xdir-windows.exe; se instala con el nombre de siempre.
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "xdir.exe"; Flags: ignoreversion
; Ícono que InitializeWizard pone a la izquierda del título de la bienvenida (solo se usa durante la instalación).
Source: "wizard-title-icon.bmp"; Flags: dontcopy

[InstallDelete]
; xtree ahora es parte de xdir: se borra el programa aparte que dejaron las versiones anteriores.
Type: files; Name: "{app}\xtree.exe"

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

[Code]
const
  WM_CLOSE = $0010;

// Cierra las ventanas de xdir abiertas (clase "xdir.main", ver CLASS_NAME en crates/xdir-windows/src/app/win.rs) para que
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

// Antes de instalar: un xtree.exe de una versión anterior que siga abierto no se podría borrar
// (ver [InstallDelete]); se lo termina. Si no está corriendo, taskkill no hace nada.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM xtree.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  CloseXdir();
  Result := True;
end;

// Ícono de xdir a la izquierda del título de la bienvenida: el título se corre a la derecha para dejarle lugar.
procedure InitializeWizard();
var
  Icon: TBitmapImage;
  Gap: Integer;
begin
  ExtractTemporaryFile('wizard-title-icon.bmp');
  Gap := ScaleX(48) + ScaleX(12);
  Icon := TBitmapImage.Create(WizardForm);
  Icon.Parent := WizardForm.WelcomePage;
  Icon.Stretch := True;
  Icon.Bitmap.LoadFromFile(ExpandConstant('{tmp}\wizard-title-icon.bmp'));
  Icon.SetBounds(WizardForm.WelcomeLabel1.Left, WizardForm.WelcomeLabel1.Top, ScaleX(48), ScaleY(48));
  WizardForm.WelcomeLabel1.Left := WizardForm.WelcomeLabel1.Left + Gap;
  WizardForm.WelcomeLabel1.Width := WizardForm.WelcomeLabel1.Width - Gap;
end;
