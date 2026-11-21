# Convert assets/startup/EILIi.mp4 into the packed splash frames the desktop
# app plays as its load screen. ffmpeg is used only as an external tool; the
# app never links a video decoder.
#
# Output: assets/startup/splash.ofsp (see assets/startup/README.md).
# Needs ffmpeg on PATH.
# Usage: scripts\gen-startup-frames.ps1
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Src = Join-Path $Root "assets\startup\EILIi.mp4"
$Out = Join-Path $Root "assets\startup\splash.ofsp"
# Native 24 fps, 736x400. -q:v 8 is a size/quality trade-off for this
# high-motion clip; animated WebP was larger than the source MP4.
$Width = [uint32]736
$Height = [uint32]400
$FpsNum = [uint32]24
$FpsDen = [uint32]1
$Version = [uint32]1
$Qv = 8

if (-not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) {
    throw "ffmpeg not found on PATH; install it to regenerate $Out"
}
if (-not (Test-Path -LiteralPath $Src)) {
    throw "missing source video: $Src"
}

$Tmp = Join-Path $env:TEMP ("ohmforge-splash-" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    $pattern = Join-Path $Tmp "frame-%03d.jpg"
    & ffmpeg -y -hide_banner -loglevel error -i $Src -map 0:v:0 -an -q:v $Qv -start_number 0 $pattern
    if ($LASTEXITCODE -ne 0) {
        throw "ffmpeg failed with exit code $LASTEXITCODE"
    }

    $frames = @(Get-ChildItem -LiteralPath $Tmp -Filter "frame-*.jpg" | Sort-Object Name)
    if ($frames.Count -eq 0) {
        throw "no JPEG frames written"
    }

    $fs = [System.IO.File]::Create($Out)
    try {
        $bw = New-Object System.IO.BinaryWriter $fs
        $bw.Write([byte[]](0x4F, 0x46, 0x53, 0x50))
        $bw.Write($Version)
        $bw.Write($Width)
        $bw.Write($Height)
        $bw.Write($FpsNum)
        $bw.Write($FpsDen)
        $bw.Write([uint32]$frames.Count)
        foreach ($frame in $frames) {
            $data = [System.IO.File]::ReadAllBytes($frame.FullName)
            if ($data.Length -lt 2 -or $data[0] -ne 0xFF -or $data[1] -ne 0xD8) {
                throw "not a JPEG: $($frame.FullName)"
            }
            $bw.Write([uint32]$data.Length)
            $bw.Write($data)
        }
        $bw.Flush()
    } finally {
        $fs.Dispose()
    }
    $size = (Get-Item -LiteralPath $Out).Length
    Write-Output "wrote $Out ($($frames.Count) frames, $size bytes)"
} finally {
    Remove-Item -LiteralPath $Tmp -Recurse -Force
}
