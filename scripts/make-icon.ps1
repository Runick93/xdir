# Genera assets\xdir.ico: carpeta oscura con borde celeste y 4 paneles cuyas esquinas exteriores cortadas
# forman una X en el espacio negativo; el panel activo en celeste (ver assets\xdir.svg).
# Uso: pwsh scripts\make-icon.ps1 [-Preview]   (-Preview deja también los PNG en assets\icon-png\)
# El .ico queda versionado; solo hace falta correrlo si cambia el diseño.
param([switch]$Preview)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot

$folder = [Drawing.Color]::FromArgb(0x1F, 0x20, 0x23)
$active = [Drawing.Color]::FromArgb(0x5B, 0xC9, 0xFB)
$pane   = [Drawing.Color]::FromArgb(0x83, 0x91, 0xA0)
$sizes  = 16, 20, 24, 32, 40, 48, 64, 256

# Panel de arriba a la izquierda en la grilla de 256: rectángulo 50..125 x 80..139 con dos esquinas
# cortadas (arriba hacia el centro y abajo hacia afuera). Los otros tres son espejos respecto del
# centro de la separación (x = 128, y = 142).
$paneTL = @(50, 80), @(94, 80), @(125, 109), @(125, 139), @(87, 139), @(50, 103)

function Render([int]$n) {
    $s = $n / 256.0
    $bmp = New-Object Drawing.Bitmap $n, $n, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([Drawing.Color]::Transparent)

    # En tamaños chicos los bordes se ajustan a píxeles enteros para que no queden borrosos.
    $small = $n -le 32
    function P([float]$v) { if ($small) { [Math]::Round($v * $s) } else { $v * $s } }

    # Carpeta: pestaña arriba a la izquierda con bajada en diagonal, y el cuerpo. $d corre el contorno
    # hacia adentro; la diagonal de la pestaña (22 x 24 en la grilla) se desplaza sobre su normal, que
    # en horizontal equivale a 0.44 * $d hacia la izquierda.
    function FolderPath([float]$d) {
        $l = (P 16) + $d; $r = (P 240) - $d; $t = (P 36) + $d; $b = (P 220) - $d; $bodyTop = (P 60) + $d
        $rad = [Math]::Max(1, [Math]::Max(1.5, 16 * $s) - $d)
        $tabEnd = (P 92) - 0.44 * $d; $slopeEnd = (P 114) - 0.44 * $d
        $path = New-Object Drawing.Drawing2D.GraphicsPath
        $path.AddArc($l, $t, 2 * $rad, 2 * $rad, 180, 90)
        $path.AddLine($l + $rad, $t, $tabEnd, $t)
        $path.AddLine($tabEnd, $t, $slopeEnd, $bodyTop)
        $path.AddArc($r - 2 * $rad, $bodyTop, 2 * $rad, 2 * $rad, 270, 90)
        $path.AddArc($r - 2 * $rad, $b - 2 * $rad, 2 * $rad, 2 * $rad, 0, 90)
        $path.AddArc($l, $b - 2 * $rad, 2 * $rad, 2 * $rad, 90, 90)
        $path.CloseFigure()
        $path
    }
    # Se pinta entera en celeste y encima, achicada el ancho del borde (3.5 de 256, al menos 1 px), en oscuro.
    $border = if ($small) { [Math]::Max(1, [Math]::Round(3.5 * $s)) } else { 3.5 * $s }
    $g.FillPath((New-Object Drawing.SolidBrush $active), (FolderPath 0))
    $g.FillPath((New-Object Drawing.SolidBrush $folder), (FolderPath $border))

    # Paneles: el de arriba a la izquierda y sus espejos. En tamaños chicos la separación
    # (6 de 256) sería de menos de un píxel: los bordes interiores se fijan con al menos 1 px.
    $gap = [Math]::Max(1, [Math]::Round(6 * $s))
    $inX = [Math]::Round(128 * $s - $gap / 2); $inY = [Math]::Round(142 * $s - $gap / 2)
    function Inner([float]$v, [float]$center, [float]$edge) {
        if (-not $small) { return P $v }
        if ($v -eq $center - 3) { return $edge }
        if ($v -eq $center + 3) { return $edge + $gap }
        P $v
    }
    foreach ($mx in $false, $true) {
        foreach ($my in $false, $true) {
            $pts = foreach ($p in $paneTL) {
                $x = if ($mx) { 256 - $p[0] } else { $p[0] }
                $y = if ($my) { 284 - $p[1] } else { $p[1] }
                New-Object Drawing.PointF (Inner $x 128 $inX), (Inner $y 142 $inY)
            }
            $c = if (-not $mx -and -not $my) { $active } else { $pane }
            $g.FillPolygon((New-Object Drawing.SolidBrush $c), [Drawing.PointF[]]$pts)
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
