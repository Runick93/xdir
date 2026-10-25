# Genera assets\xdir.ico: carpeta oscura con 4 paneles, el activo en celeste (ver assets\xdir.svg).
# Uso: pwsh scripts\make-icon.ps1 [-Preview]   (-Preview deja también los PNG en assets\icon-png\)
# El .ico queda versionado; solo hace falta correrlo si cambia el diseño.
param([switch]$Preview)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot

$folder = [Drawing.Color]::FromArgb(0x1B, 0x1C, 0x1E)
$active = [Drawing.Color]::FromArgb(0x5B, 0xC0, 0xFF)
$pane   = [Drawing.Color]::FromArgb(0x5E, 0x69, 0x76)
$sizes  = 16, 20, 24, 32, 40, 48, 64, 256

function RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object Drawing.Drawing2D.GraphicsPath
    $d = [Math]::Min(2 * $r, [Math]::Min($w, $h))
    if ($d -le 0) { $p.AddRectangle((New-Object Drawing.RectangleF $x, $y, $w, $h)); return $p }
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    $p
}

function Render([int]$n) {
    $s = $n / 256.0
    $bmp = New-Object Drawing.Bitmap $n, $n, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([Drawing.Color]::Transparent)

    # Carpeta: pestaña arriba a la izquierda con bajada en diagonal, y el cuerpo.
    # Coordenadas en una grilla de 256; en tamaños chicos se ajustan a píxeles enteros.
    $small = $n -le 32
    function P([float]$v) { if ($small) { [Math]::Round($v * $s) } else { $v * $s } }
    $l = P 16; $r = P 240; $t = P 24; $b = P 232; $bodyTop = P 54
    $rad = [Math]::Max(1.5, 28 * $s)
    $tabEnd = P 113; $slopeEnd = P 144
    $path = New-Object Drawing.Drawing2D.GraphicsPath
    $path.AddArc($l, $t, 2 * $rad, 2 * $rad, 180, 90)
    $path.AddLine($l + $rad, $t, $tabEnd, $t)
    $path.AddLine($tabEnd, $t, $slopeEnd, $bodyTop)
    $path.AddArc($r - 2 * $rad, $bodyTop, 2 * $rad, 2 * $rad, 270, 90)
    $path.AddArc($r - 2 * $rad, $b - 2 * $rad, 2 * $rad, 2 * $rad, 0, 90)
    $path.AddArc($l, $b - 2 * $rad, 2 * $rad, 2 * $rad, 90, 90)
    $path.CloseFigure()
    $g.FillPath((New-Object Drawing.SolidBrush $folder), $path)

    # Paneles 2x2. En tamaños chicos: bordes enteros y al menos 1 px de separación.
    $xs = (P 36), (P 124), (P 134), (P 222)
    $ys = (P 77), (P 142), (P 151), (P 216)
    if ($small) {
        if ($xs[2] - $xs[1] -lt 1) { $xs[2] = $xs[1] + 1 }
        if ($ys[2] - $ys[1] -lt 1) { $ys[2] = $ys[1] + 1 }
    }
    $pr = if ($n -le 20) { 0.6 } else { 12 * $s }
    foreach ($i in 0, 1) {
        foreach ($j in 0, 1) {
            $x0 = $xs[2 * $j]; $x1 = $xs[2 * $j + 1]; $y0 = $ys[2 * $i]; $y1 = $ys[2 * $i + 1]
            $c = if ($i -eq 0 -and $j -eq 0) { $active } else { $pane }
            $g.FillPath((New-Object Drawing.SolidBrush $c), (RoundRect $x0 $y0 ($x1 - $x0) ($y1 - $y0) $pr))
        }
    }
    $g.Dispose()
    $ms = New-Object IO.MemoryStream
    $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , $ms.ToArray()
}

$images = foreach ($n in $sizes) { , (Render $n) }

if ($Preview) {
    $dir = Join-Path $root "assets\icon-png"
    New-Item -ItemType Directory -Force $dir | Out-Null
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        [IO.File]::WriteAllBytes((Join-Path $dir "xdir-$($sizes[$i]).png"), $images[$i])
    }
}

# ICO con entradas PNG (soportadas desde Windows Vista, también por rc.exe).
$out = New-Object IO.MemoryStream
$w = New-Object IO.BinaryWriter $out
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $n = $sizes[$i]; $len = $images[$i].Length
    $dim = if ($n -ge 256) { 0 } else { $n }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$len); $w.Write([uint32]$offset)
    $offset += $len
}
foreach ($img in $images) { $w.Write($img) }
$w.Flush()
$ico = Join-Path $root "assets\xdir.ico"
[IO.File]::WriteAllBytes($ico, $out.ToArray())
Write-Host "Ícono: $ico ($($out.Length) bytes, tamaños: $($sizes -join ', '))"
