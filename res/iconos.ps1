# Genera los iconos .ico de res/ (ejecutar desde la raíz del repo):
#   powershell -ExecutionPolicy Bypass -File res\iconos.ps1
Add-Type -AssemblyName System.Drawing

function New-Png([int]$size, [string]$bg, [bool]$round) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $brush = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($bg))
    $m = [math]::Max(1, $size / 16)
    if ($round) {
        $g.FillEllipse($brush, $m * 0.5, $m * 0.5, $size - $m, $size - $m)
    } else {
        $r = $size * 0.22
        $p = New-Object System.Drawing.Drawing2D.GraphicsPath
        $w = $size - 1
        $p.AddArc(0, 0, $r, $r, 180, 90); $p.AddArc($w - $r, 0, $r, $r, 270, 90)
        $p.AddArc($w - $r, $w - $r, $r, $r, 0, 90); $p.AddArc(0, $w - $r, $r, $r, 90, 90)
        $p.CloseFigure(); $g.FillPath($brush, $p)
    }
    # trazo tipo "pulso": plano, pico, valle y plano
    $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::White), ([math]::Max(1.6, $size / 11))
    $pen.LineJoin = 'Round'; $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
    $s = $size
    $pts = @(
        (New-Object System.Drawing.PointF ($s * 0.17), ($s * 0.56)),
        (New-Object System.Drawing.PointF ($s * 0.37), ($s * 0.56)),
        (New-Object System.Drawing.PointF ($s * 0.47), ($s * 0.26)),
        (New-Object System.Drawing.PointF ($s * 0.58), ($s * 0.76)),
        (New-Object System.Drawing.PointF ($s * 0.66), ($s * 0.50)),
        (New-Object System.Drawing.PointF ($s * 0.83), ($s * 0.50))
    )
    $g.DrawLines($pen, [System.Drawing.PointF[]]$pts)
    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return , $ms.ToArray()
}

function Write-Ico([string]$path, [string]$bg, [bool]$round, [int[]]$sizes) {
    $imgs = $sizes | ForEach-Object { , (New-Png $_ $bg $round) }
    $out = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter $out
    $bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $sz = $sizes[$i]; $d = [byte]($(if ($sz -ge 256) { 0 } else { $sz }))
        $bw.Write($d); $bw.Write($d); $bw.Write([byte]0); $bw.Write([byte]0)
        $bw.Write([uint16]1); $bw.Write([uint16]32)
        $bw.Write([uint32]$imgs[$i].Length); $bw.Write([uint32]$offset)
        $offset += $imgs[$i].Length
    }
    foreach ($img in $imgs) { $bw.Write([byte[]]$img) }
    $bw.Flush()
    [System.IO.File]::WriteAllBytes($path, $out.ToArray())
}

$dir = Split-Path -Parent $MyInvocation.MyCommand.Path
Write-Ico (Join-Path $dir 'icono.ico')   '#2b4a6f' $false @(16, 24, 32, 48, 64, 256)
Write-Ico (Join-Path $dir 'verde.ico')   '#2e7d4f' $true  @(16, 20, 24, 32)
Write-Ico (Join-Path $dir 'naranja.ico') '#c7741a' $true  @(16, 20, 24, 32)
Write-Ico (Join-Path $dir 'rojo.ico')    '#b8322a' $true  @(16, 20, 24, 32)
Write-Ico (Join-Path $dir 'gris.ico')    '#6b6f76' $true  @(16, 20, 24, 32)
