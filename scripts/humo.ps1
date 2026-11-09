# Prueba de humo de xdir en Windows: abre el exe compilado con `--features pruebas`, lo maneja por
# WM_COPYDATA (layouts, paleta, panel de favoritos, xtree, aviso), captura la ventana y la cierra.
# Falla si no aparece la ventana, si un comando no responde 1, si el proceso muere o si no sale con 0.
# Una captura vacía solo avisa (en runners sin GPU puede salir en blanco).
# Uso: pwsh scripts/humo.ps1 [-Exe target\debug\xdir.exe] [-Out humo]
param(
    [string]$Exe = 'target\debug\xdir.exe',
    [string]$Out = 'humo'
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing
# En pwsh 7 Bitmap/Graphics están en System.Drawing.Common y Color en System.Drawing.Primitives:
# se referencian por la ruta real de cada ensamblado cargado.
$refs = @(
    [System.Drawing.Bitmap].Assembly.Location,
    [System.Drawing.Graphics].Assembly.Location,
    [System.Drawing.Color].Assembly.Location
) | Select-Object -Unique
Add-Type -ReferencedAssemblies $refs -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class Humo {
    [StructLayout(LayoutKind.Sequential)]
    struct COPYDATASTRUCT { public IntPtr dwData; public int cbData; public IntPtr lpData; }

    [StructLayout(LayoutKind.Sequential)]
    struct RECT { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr wp, ref COPYDATASTRUCT lp,
        uint flags, uint timeoutMs, out IntPtr result);
    [DllImport("user32.dll")]
    static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")]
    static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")]
    static extern bool GetWindowRect(IntPtr h, out RECT r);

    const uint WM_COPYDATA = 0x004A;
    const uint WM_CLOSE = 0x0010;
    const uint SMTO_ABORTIFHUNG = 0x0002;

    public static IntPtr Find() { return FindWindowW("xdir.main", null); }

    // Manda `text` por WM_COPYDATA (UTF-16 con NUL final) y devuelve lo que contesta la ventana,
    // o -1 si no contestó en 10 s (ventana colgada).
    public static long Send(IntPtr hwnd, string text) {
        IntPtr buf = Marshal.StringToHGlobalUni(text + "\0");
        try {
            COPYDATASTRUCT cds = new COPYDATASTRUCT();
            cds.dwData = IntPtr.Zero;
            cds.cbData = (text.Length + 1) * 2;
            cds.lpData = buf;
            IntPtr res;
            IntPtr ok = SendMessageTimeoutW(hwnd, WM_COPYDATA, IntPtr.Zero, ref cds, SMTO_ABORTIFHUNG, 10000, out res);
            if (ok == IntPtr.Zero) return -1;
            return res.ToInt64();
        } finally {
            Marshal.FreeHGlobal(buf);
        }
    }

    public static void Close(IntPtr hwnd) { PostMessageW(hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero); }

    // Guarda la ventana en `path` (PNG) y devuelve cuántos colores distintos tiene (hasta 64).
    public static int Capture(IntPtr hwnd, string path) {
        RECT r;
        if (!GetWindowRect(hwnd, out r)) return 0;
        int w = r.Right - r.Left, h = r.Bottom - r.Top;
        if (w <= 0 || h <= 0) return 0;
        using (Bitmap bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
            using (Graphics g = Graphics.FromImage(bmp)) {
                IntPtr hdc = g.GetHdc();
                try { PrintWindow(hwnd, hdc, 2); } finally { g.ReleaseHdc(hdc); }
            }
            bmp.Save(path, ImageFormat.Png);
            HashSet<int> colors = new HashSet<int>();
            for (int y = 0; y < h && colors.Count < 64; y += 3)
                for (int x = 0; x < w && colors.Count < 64; x += 3)
                    colors.Add(bmp.GetPixel(x, y).ToArgb());
            return colors.Count;
        }
    }
}
'@

function Fallar([string]$msg) {
    Write-Error $msg -ErrorAction Continue
    exit 1
}

# Carpeta de trabajo (3 archivos y 2 subcarpetas) y configuración aislada en %APPDATA%\xdir.
$base = Join-Path ([IO.Path]::GetTempPath()) ("xdir-humo-" + [guid]::NewGuid().ToString('N'))
$work = Join-Path $base 'trabajo'
$conf = Join-Path $base 'config'
New-Item -ItemType Directory -Force -Path $work, $conf | Out-Null
1..3 | ForEach-Object { Set-Content -Path (Join-Path $work "archivo$_.txt") -Value "contenido $_" }
1..2 | ForEach-Object { New-Item -ItemType Directory -Force -Path (Join-Path $work "carpeta$_") | Out-Null }
$env:APPDATA = $conf

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$exePath = (Resolve-Path $Exe).Path
$proc = Start-Process -FilePath $exePath -ArgumentList "`"$work`"" -PassThru
$null = $proc.Handle  # fija el código de salida aunque el proceso termine antes de leerlo

# Espera la ventana (hasta 30 s).
$hwnd = [IntPtr]::Zero
$limite = (Get-Date).AddSeconds(30)
while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $limite) {
    if ($proc.HasExited) { Fallar "el proceso terminó antes de abrir la ventana (código $($proc.ExitCode))" }
    $hwnd = [Humo]::Find()
    if ($hwnd -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 250 }
}
if ($hwnd -eq [IntPtr]::Zero) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Fallar 'no apareció la ventana'
}
Write-Host "ventana encontrada: $hwnd"
Start-Sleep -Milliseconds 500

$script:primero = 'cmd layout_4'
function Mandar([string]$texto) {
    $r = [Humo]::Send($hwnd, $texto)
    if ($texto -eq $script:primero) {
        # El primer comando puede llegar antes de que la ventana procese mensajes: hasta 5 intentos.
        for ($i = 1; $i -lt 5 -and $r -ne 1; $i++) {
            Start-Sleep -Milliseconds 500
            $r = [Humo]::Send($hwnd, $texto)
        }
    }
    Write-Host "mandado '$texto' -> $r"
    if ($r -ne 1) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        Fallar "no responde a '$texto' (devolvió $r; -1 = sin respuesta en 10 s)"
    }
    if ($proc.HasExited) { Fallar "el proceso murió después de '$texto' (código $($proc.ExitCode))" }
    Start-Sleep -Milliseconds 500
}

Mandar 'cmd layout_4'
Mandar 'cmd layout_1'
Mandar 'cmd paleta'
Mandar 'close'
Mandar 'cmd xtree'
Start-Sleep -Seconds 3
if ($proc.HasExited) { Fallar "el proceso murió con xtree (código $($proc.ExitCode))" }
Mandar 'cmd xtree'
Mandar 'notice humo'

# Captura.
$png = Join-Path $Out 'humo.png'
$colores = [Humo]::Capture($hwnd, (Join-Path (Resolve-Path $Out).Path 'humo.png'))
Write-Host "captura: $png ($colores colores distintos)"
if ($colores -lt 8) {
    Write-Warning 'la captura salió vacía'
    Write-Host '::warning::la captura salió vacía'
}

# Cierre limpio.
[Humo]::Close($hwnd)
if (-not $proc.WaitForExit(15000)) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Fallar 'xdir no terminó a los 15 s de WM_CLOSE'
}
if ($proc.ExitCode -ne 0) { Fallar "xdir terminó con código $($proc.ExitCode)" }
Write-Host 'prueba de humo OK'
exit 0
