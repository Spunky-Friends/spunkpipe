# Installs SpunkPipe on Windows.
#
#   & ([scriptblock]::Create((irm https://github.com/Spunky-Friends/spunkpipe/releases/latest/download/install.ps1))) -Invite 'spunkpipe://...'
#
param(
  [string]$Invite = "",
  [string]$BaseUrl = $(if ($env:SPUNKPIPE_BASE_URL) { $env:SPUNKPIPE_BASE_URL } else { "https://github.com/Spunky-Friends/spunkpipe/releases/latest/download" }),
  [string]$InstallDir = $(Join-Path $env:LOCALAPPDATA "Programs\SpunkPipe")
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") { throw "No SpunkPipe build for $($env:PROCESSOR_ARCHITECTURE)" }
$target = "x86_64-pc-windows-msvc"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("spunkpipe-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $version = (Invoke-RestMethod "$BaseUrl/VERSION").ToString().Trim()
  if ($version -notmatch '^[0-9.]+$') { throw "Bad version from server: $version" }
  $archive = "spunkpipe-$version-$target.zip"

  Write-Host "Downloading SpunkPipe $version..."
  $sums = (Invoke-RestMethod "$BaseUrl/SHA256SUMS").ToString()
  Invoke-WebRequest "$BaseUrl/$archive" -OutFile (Join-Path $tmp $archive)

  $want = ($sums -split "`n" | Where-Object { $_ -match "  $([regex]::Escape($archive))\s*$" } | Select-Object -First 1)
  if (-not $want) { throw "No checksum for $archive" }
  $want = ($want -split "\s+")[0].ToLower()
  $got = (Get-FileHash (Join-Path $tmp $archive) -Algorithm SHA256).Hash.ToLower()
  if ($got -ne $want) { throw "Checksum mismatch for ${archive}: refusing to install" }

  # Stop a running copy so its files can be replaced.
  Get-Process spunkpipe-tray -ErrorAction SilentlyContinue | Stop-Process -Force
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  Expand-Archive -Path (Join-Path $tmp $archive) -DestinationPath $InstallDir -Force
  Write-Host "Installed to $InstallDir"

  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  if (($userPath -split ";") -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
    Write-Host "Added $InstallDir to your PATH (open a new terminal to use it)."
  }

  if ($Invite) {
    & (Join-Path $InstallDir "spunkpipe.exe") join $Invite
    Start-Process (Join-Path $InstallDir "spunkpipe-tray.exe")
  } else {
    Write-Host "Next: spunkpipe join '<the invite you were sent>'"
  }
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
