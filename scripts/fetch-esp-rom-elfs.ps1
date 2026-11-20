# Download Espressif's ROM ELFs (esp-rom-elfs, Apache-2.0) for the ESP32-C3
# ROM boot tests. The ROM code is Espressif's and not open source, so it is
# never committed: this fetches a pinned release, checks its SHA-256 and
# unpacks it outside target\ (build caches restore target\).
# It also fetches the ESP-IDF second-stage bootloader that espflash ships for
# the ESP32-C3 (Espressif's code, Apache-2.0, built from ESP-IDF; pinned to an
# espflash release and checked by SHA-256), which tests\esp_hal_blinky.rs
# writes at flash offset 0 as espflash does.
#
# Output: $env:OF_ESP_ROM_DIR, else <repo>\.cache\esp-rom-elfs\<release>
# The tests look in the same places (crates\of-soc-esp32c3\tests\common\mod.rs).
# Usage: scripts\fetch-esp-rom-elfs.ps1
$ErrorActionPreference = "Stop"

# Keep in step with of_soc_esp32c3::rom::{ROM_ELFS_RELEASE, ROM_ELFS_TARBALL_SHA256}.
$Release = "20260528"
$Sha256 = "caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46"
$Root = Split-Path -Parent $PSScriptRoot
$Out = if ($env:OF_ESP_ROM_DIR) { $env:OF_ESP_ROM_DIR } else { Join-Path $Root ".cache\esp-rom-elfs\$Release" }
$Tarball = "esp-rom-elfs-$Release.tar.gz"
$Url = "https://github.com/espressif/esp-rom-elfs/releases/download/$Release/$Tarball"
# Keep in step with of_soc_esp32c3::bootloader::{ESPFLASH_RELEASE, C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256}.
$Espflash = "v4.7.0"
$BlFile = "espflash-$Espflash-esp32c3-bootloader.bin"
$BlSha256 = "7c04141e1fd8da5a391e8004ffe3d8ee8e20eeb5baf9bd3abc05c03a17b1ed09"
$BlUrl = "https://raw.githubusercontent.com/esp-rs/espflash/$Espflash/espflash/resources/bootloaders/esp32c3-bootloader.bin"

function Get-Checked($Uri, $File, $Want) {
    Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $File
    $Got = (Get-FileHash -Algorithm SHA256 $File).Hash.ToLowerInvariant()
    if ($Got -ne $Want) { throw "$(Split-Path -Leaf $File) has SHA-256 $Got, expected $Want" }
}

New-Item -ItemType Directory -Force $Out | Out-Null
if (Test-Path (Join-Path $Out "esp32c3_rev3_rom.elf")) {
    Write-Output "esp-rom-elfs $Release already in $Out"
} else {
    $Tmp = Join-Path ([System.IO.Path]::GetTempPath()) $Tarball
    try {
        Get-Checked $Url $Tmp $Sha256
        # tar.exe ships with Windows 10 1803 and later.
        tar -xzf $Tmp -C $Out
        if ($LASTEXITCODE -ne 0) { throw "could not unpack $Tarball" }
    } finally {
        Remove-Item -Force $Tmp -ErrorAction SilentlyContinue
    }
    Write-Output "unpacked esp-rom-elfs $Release into $Out"
}
$BlOut = Join-Path $Out $BlFile
if (Test-Path $BlOut) {
    Write-Output "$BlFile already in $Out"
} else {
    $Tmp = Join-Path ([System.IO.Path]::GetTempPath()) $BlFile
    try {
        Get-Checked $BlUrl $Tmp $BlSha256
        Move-Item -Force $Tmp $BlOut
    } finally {
        Remove-Item -Force $Tmp -ErrorAction SilentlyContinue
    }
    Write-Output "fetched $BlFile into $Out"
}
Write-Output "run: cargo test -p of-soc-esp32c3 --test rom_boot --test esp_hal_blinky"
