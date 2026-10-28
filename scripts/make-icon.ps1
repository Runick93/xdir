# Genera assets\xdir.ico: carpeta oscura con borde celeste y 4 paneles cuyas esquinas exteriores cortadas
# forman una X en el espacio negativo; el panel activo en celeste (ver assets\xdir.svg).
# Uso: pwsh scripts\make-icon.ps1 [-Preview]   (-Preview deja también los PNG en assets\icon-png\)
# El .ico queda versionado; solo hace falta correrlo si cambia el diseño.
param([switch]$Preview)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot

$folder = [Drawing.Color]::FromArgb(0x18, 0x1A, 0x1C)
$active = [Drawing.Color]::FromArgb(0x5A, 0xCA, 0xFD)
$pane   = [Drawing.Color]::FromArgb(0xDC, 0xE3, 0xEC)
$sizes  = 16, 20, 24, 32, 40, 48, 64, 256
# La carpeta ocupa 224 x 180 de los 256 de la grilla; se agranda, centrada, hasta casi el ancho del cuadro como la carpeta de Windows, para que en la
# barra de tareas no se vea más chica que los íconos que llenan el cuadro.
$zoom = 1.12

# Brazo de arriba a la izquierda en la grilla de 256: una diagonal ancha de la esquina (49, 82) al centro,
# con la punta de afuera recta (arriba y a la izquierda) y la de adentro cortada en horizontal y vertical. Los otros tres son espejos respecto del
# centro de la separación (x = 128, y = 141): 8 de ancho entre columnas y 6 entre filas.
$paneTL = @(49, 82), @(71, 82), @(124, 132), @(124, 138), @(104, 138), @(49, 92)
# En 16 a 24 px los brazos finos se cierran sobre sí mismos (los bordes interiores se fijan a píxeles
# enteros): ahí se usan brazos más gruesos, con las esquinas cortadas.
$paneTLSmall = @(46, 78), @(92, 78), @(124, 110), @(124, 138), @(84, 138), @(46, 100)

function Render([int]$n) {
    $s = $n / 256.0 * $zoom
    $bmp = New-Object Drawing.Bitmap $n, $n, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([Drawing.Color]::Transparent)

    # En tamaños chicos los bordes se ajustan a píxeles enteros para que no queden borrosos.
    $small = $n -le 32
    function P([float]$v) { $x = ($v - 128) * $s + $n / 2; if ($small) { [Math]::Round($x) } else { $x } }

    # Carpeta: pestaña arriba a la izquierda con bajada en diagonal, y el cuerpo. $d corre el contorno
    # hacia adentro; la diagonal de la pestaña (18 x 22 en la grilla) se desplaza sobre su normal, que
    # en horizontal equivale a 0.47 * $d hacia la izquierda.
    function FolderPath([float]$d) {
        $l = (P 16) + $d; $r = (P 240) - $d; $t = (P 38) + $d; $b = (P 218) - $d; $bodyTop = (P 60) + $d
        $rad = [Math]::Max(1, [Math]::Max(1.5, 18 * $s) - $d)
        $tabEnd = (P 94) - 0.47 * $d; $slopeEnd = (P 112) - 0.47 * $d
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
    # Se pinta entera en celeste y encima, achicada el ancho del borde (4.75 de 256, al menos 1 px), en oscuro.
    $border = if ($small) { [Math]::Max(1, [Math]::Round(4.75 * $s)) } else { 4.75 * $s }
    $g.FillPath((New-Object Drawing.SolidBrush $active), (FolderPath 0))
    $g.FillPath((New-Object Drawing.SolidBrush $folder), (FolderPath $border))

    # Paneles: el de arriba a la izquierda y sus espejos. En tamaños chicos la separación
    # (8 x 6 de 256) sería de menos de un píxel: los bordes interiores se fijan con al menos 1 px.
    $gapX = [Math]::Max(1, [Math]::Round(8 * $s)); $gapY = [Math]::Max(1, [Math]::Round(6 * $s))
    $inX = [Math]::Round($n / 2 - $gapX / 2); $inY = [Math]::Round($n / 2 + 13 * $s - $gapY / 2)
    function Inner([float]$v, [float]$center, [float]$half, [float]$edge, [float]$gap) {
        if (-not $small) { return P $v }
        if ($v -eq $center - $half) { return $edge }
        if ($v -eq $center + $half) { return $edge + $gap }
        P $v
    }
    # Esquinas de los paneles redondeadas (3 de 256); por debajo de 40 px no se notan y solo
    # ablandarían los bordes, así que quedan en punta.
    $corner = if ($n -ge 40) { 3 * $s } else { 0 }
    function PanePath([Drawing.PointF[]]$pts) {
        $path = New-Object Drawing.Drawing2D.GraphicsPath
        for ($i = 0; $i -lt $pts.Count; $i++) {
            $v = $pts[$i]; $prev = $pts[($i + $pts.Count - 1) % $pts.Count]; $next = $pts[($i + 1) % $pts.Count]
            $lp = [Math]::Sqrt(($prev.X - $v.X) * ($prev.X - $v.X) + ($prev.Y - $v.Y) * ($prev.Y - $v.Y))
            $ln = [Math]::Sqrt(($next.X - $v.X) * ($next.X - $v.X) + ($next.Y - $v.Y) * ($next.Y - $v.Y))
            $a = New-Object Drawing.PointF ($v.X + $corner * ($prev.X - $v.X) / $lp), ($v.Y + $corner * ($prev.Y - $v.Y) / $lp)
            $b = New-Object Drawing.PointF ($v.X + $corner * ($next.X - $v.X) / $ln), ($v.Y + $corner * ($next.Y - $v.Y) / $ln)
            $path.AddBezier($a, $v, $v, $b)
        }
        $path.CloseFigure()
        $path
    }
    foreach ($mx in $false, $true) {
        foreach ($my in $false, $true) {
            $pts = foreach ($p in $(if ($n -le 24) { $paneTLSmall } else { $paneTL })) {
                $x = if ($mx) { 256 - $p[0] } else { $p[0] }
                $y = if ($my) { 282 - $p[1] } else { $p[1] }
                New-Object Drawing.PointF (Inner $x 128 4 $inX $gapX), (Inner $y 141 3 $inY $gapY)
            }
            $c = if (-not $mx -and -not $my) { $active } else { $pane }
            $g.FillPath((New-Object Drawing.SolidBrush $c), (PanePath $pts))
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
