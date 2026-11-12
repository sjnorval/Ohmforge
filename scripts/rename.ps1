<#
.SYNOPSIS
  Rename the placeholder project name "Ohmforge" and crate prefix "of-".
.EXAMPLE
  ./scripts/rename.ps1 -Name sparkbench -Prefix sb
  Review with `git diff` before committing.
#>
param(
  [Parameter(Mandatory)] [string] $Name,    # lower-case, e.g. sparkbench
  [Parameter(Mandatory)] [string] $Prefix   # crate prefix without dash, e.g. sb
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$title = (Get-Culture).TextInfo.ToTitleCase($Name)
$files = Get-ChildItem $root -Recurse -File -Include *.rs,*.toml,*.md,*.yml,*.ps1 |
  Where-Object { $_.FullName -notmatch '\\(target|\.git)\\' -and $_.Name -ne 'DESIGN_AND_PLAN.md' }
foreach ($f in $files) {
  $t = Get-Content $f.FullName -Raw
  $n = $t -replace 'Ohmforge', $title -replace 'ohmforge', $Name `
          -replace '\bof-', "$Prefix-" -replace '\bof_', "${Prefix}_"
  if ($n -ne $t) { Set-Content -Path $f.FullName -Value $n -NoNewline }
}
Get-ChildItem (Join-Path $root 'crates') -Directory -Filter 'of-*' |
  ForEach-Object { Rename-Item $_.FullName ($_.Name -replace '^of-', "$Prefix-") }
Write-Host "Renamed to $title ($Prefix-*). Run cargo check, then review git diff."
