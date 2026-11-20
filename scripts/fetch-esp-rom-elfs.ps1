# Download Espressif's ROM ELFs (esp-rom-elfs, Apache-2.0) for the ESP32-C3
# ROM boot tests. The ROM code is Espressif's and not open source, so it is
# never committed: this fetches a pinned release, checks its SHA-256 and
# unpacks it outside target\ (build caches restore target\).
#
# Output: $env:OF_ESP_ROM_DIR, else <repo>\.cache\esp-rom-elfs\<release>
# The tests look in the same places (crates\of-soc-esp32c3\tests\rom_boot.rs).
# Usage: scripts\fetch-esp-rom-elfs.ps1
$ErrorActionPreference = "Stop"

# Keep in step with of_soc_esp32c3::rom::{ROM_ELFS_RELEASE, ROM_ELFS_TARBALL_SHA256}.
$Release = "20260528"
$Sha256 = "caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46"
$Root = Split-Path -Parent $PSScriptRoot
$Out = if ($env:OF_ESP_ROM_DIR) { $env:OF_ESP_ROM_DIR } else { Join-Path $Root ".cache\esp-rom-elfs\$Release" }
$Tarball = "esp-rom-elfs-$Release.tar.gz"
$Url = "https://github.com/espressif/esp-rom-elfs/releases/download/$Release/$Tarball"

if (Test-Path (Join-Path $Out "esp32c3_rev3_rom.elf")) {
    Write-Output "esp-rom-elfs $Release already in $Out"; exit 0
}
New-Item -ItemType Directory -Force $Out | Out-Null
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) $Tarball
try {
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Tmp
    $Got = (Get-FileHash -Algorithm SHA256 $Tmp).Hash.ToLowerInvariant()
    if ($Got -ne $Sha256) { throw "$Tarball has SHA-256 $Got, expected $Sha256" }
    # tar.exe ships with Windows 10 1803 and later.
    tar -xzf $Tmp -C $Out
    if ($LASTEXITCODE -ne 0) { throw "could not unpack $Tarball" }
} finally {
    Remove-Item -Force $Tmp -ErrorAction SilentlyContinue
}
Write-Output "unpacked esp-rom-elfs $Release into $Out"
Write-Output "run: cargo test -p of-soc-esp32c3 --test rom_boot"
