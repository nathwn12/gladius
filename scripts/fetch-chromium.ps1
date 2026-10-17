<#
.SYNOPSIS
  Provisions gladius's OWN pinned Chrome-for-Testing headless-shell (win64).

.DESCRIPTION
  Mirrors scripts/fetch-engine.ps1 in shape and discipline. Resolves one
  PINNED Chrome-for-Testing build (a version string, never a channel name),
  downloads the chrome-headless-shell zip, verifies the zip AND every
  extracted file against scripts/chromium-manifest.json (whose sha256 values
  were computed BY US from the real downloaded files - the upstream CfT JSON
  endpoints carry no sha256 field), and extracts atomically into an owned
  cache root. Re-verifies hashes on reuse and refuses on mismatch.

  Owned paths ONLY: the cache root below plus the system temp dir. This
  script never reads, writes, or launches an ambient or shared browser and
  never touches the user's installed Brave/Edge/Chrome profiles.

  Recovery: delete the cache dir shown below (or re-run with -Force) and run
  this script again. The Rust engine swap that consumes this cache is a
  later pass; until then the absence error text names this script.
#>
param(
  [string]$SourcePath = '',
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Pin (keep in lockstep with scripts/chromium-manifest.json).
$Version = '155.0.8059.84'
$Platform = 'win64'
$AssetName = 'chrome-headless-shell-win64.zip'
$AssetUrl = 'https://storage.googleapis.com/chrome-for-testing-public/155.0.8059.84/win64/chrome-headless-shell-win64.zip'
$ExpectedZipSha256 = '5ca590a4968864e219ea25ccd134597ef8898a733a4698c07fc3e8b2c480fef4'
$ExpectedZipSize = 123681250
$ZipShort8 = $ExpectedZipSha256.Substring(0, 8)
$ManifestName = 'chromium-manifest.json'
$CacheOverrideEnv = 'GLADIUS_CHROMIUM_CACHE'
$InnerRoot = 'chrome-headless-shell-win64'
$ExeRel = "$InnerRoot/chrome-headless-shell.exe"

function Fail-Absent([string]$Reason) {
  Write-Host $Reason
  Write-Host ('chrome-headless-shell is absent. Provision it with: powershell -ExecutionPolicy Bypass -File scripts\fetch-chromium.ps1')
  Write-Host 'Gladius never falls back to an ambient or shared browser, and never touches the installed Brave/Edge/Chrome.'
  exit 1
}

$ManifestPath = Join-Path $PSScriptRoot $ManifestName
if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
  Fail-Absent ("scripts\fetch-chromium.ps1: manifest missing: {0}; wrote nothing." -f $ManifestPath)
}
$Manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
$Browser = $Manifest.browser
if ($Browser.version -ne $Version -or $Browser.asset.url -ne $AssetUrl -or
    $Browser.asset.sha256.ToLowerInvariant() -ne $ExpectedZipSha256.ToLowerInvariant() -or
    [long]$Browser.asset.size -ne [long]$ExpectedZipSize) {
  Write-Host 'scripts\fetch-chromium.ps1: manifest drift: browser.version/asset.url/sha256/size do not match the pin constants at the top of this script; update both in lockstep; wrote nothing.'
  exit 1
}

$CacheOverrideItem = Get-Item -LiteralPath ("env:" + $CacheOverrideEnv) -ErrorAction SilentlyContinue
$CacheRoot = if ($null -ne $CacheOverrideItem) { $CacheOverrideItem.Value } else { '' }
if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
  if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    Fail-Absent ('scripts\fetch-chromium.ps1: LOCALAPPDATA is unset and {0} is unset; set one of them; wrote nothing.' -f $CacheOverrideEnv)
  }
  $CacheRoot = Join-Path $env:LOCALAPPDATA 'gladius\chromium'
}
$CacheDir = Join-Path $CacheRoot ("{0}-{1}" -f $Version, $ZipShort8)
$ExePath = Join-Path $CacheDir $ExeRel.Replace('/', '\')

function Get-RelFiles([string]$Dir) {
  $out = @()
  foreach ($f in Get-ChildItem -LiteralPath $Dir -Recurse -File) {
    $rel = $f.FullName.Substring($Dir.Length + 1).Replace('\', '/')
    $out += [pscustomobject]@{ Rel = $rel; Full = $f.FullName; Size = $f.Length }
  }
  return $out
}

function Test-PinnedBrowserPresent {
  if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { return $false }
  $want = @{}
  foreach ($e in $Browser.files) { $want[$e.name] = $e }
  $actual = Get-RelFiles $CacheDir
  if ($actual.Count -ne $want.Count) {
    Write-Host ('check file set: found={0} expected={1}; mismatch.' -f $actual.Count, $want.Count)
    return $false
  }
  $bad = 0
  $bytes = [long]0
  foreach ($a in $actual) {
    $e = $want[$a.Rel]
    if ($null -eq $e) {
      Write-Host ('check extra file not in manifest: {0}' -f $a.Rel)
      $bad++
      continue
    }
    $bytes += [long]$a.Size
    if ([long]$a.Size -ne [long]$e.size) {
      Write-Host ('check {0}: size mismatch computed={1} expected={2}' -f $a.Rel, $a.Size, $e.size)
      $bad++
      continue
    }
    $h = (Get-FileHash -Algorithm SHA256 -LiteralPath $a.Full).Hash.ToLowerInvariant()
    if ($h -ne $e.sha256.ToLowerInvariant()) {
      Write-Host ('check {0}: sha256 mismatch computed={1} expected={2}' -f $a.Rel, $h, $e.sha256)
      $bad++
    }
  }
  $exeEntry = $want[$ExeRel]
  $exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ExePath).Hash.ToLowerInvariant()
  $exeLen = (Get-Item -LiteralPath $ExePath).Length
  Write-Host ('check {0}: computed={1} expected={2} size={3} expected-size={4}' -f $ExeRel, $exeHash, $exeEntry.sha256, $exeLen, $exeEntry.size)
  Write-Host ('check tree: files={0} bytes={1} mismatches={2}' -f $actual.Count, $bytes, $bad)
  return ($bad -eq 0)
}

$present = Test-PinnedBrowserPresent
if ($present) {
  Write-Host ('already present, verified: {0} matches the pin; nothing changed.' -f $ExePath)
  exit 0
}

if ((Test-Path -LiteralPath $CacheDir) -and -not $Force) {
  Write-Host ('scripts\fetch-chromium.ps1: REFUSING: existing cache at {0} failed verification (absent or hash/size mismatch); wrote nothing.' -f $CacheDir)
  Write-Host ('Delete that directory, or re-run with -Force to re-provision, then run scripts\fetch-chromium.ps1 again.')
  exit 1
}
if ($Force -and (Test-Path -LiteralPath $CacheDir)) {
  Write-Host ('-Force: removing existing cache at {0}' -f $CacheDir)
  Remove-Item -LiteralPath $CacheDir -Recurse -Force
}

$ZipPath = $SourcePath
$DownloadedTemp = $false
if ([string]::IsNullOrWhiteSpace($ZipPath)) {
  $ZipPath = Join-Path ([System.IO.Path]::GetTempPath()) $AssetName
  Write-Host ("downloading {0} -> {1}" -f $AssetUrl, $ZipPath)
  try {
    Invoke-WebRequest -Uri $AssetUrl -OutFile $ZipPath
  } catch {
    Fail-Absent ('scripts\fetch-chromium.ps1: download failed: {0}; wrote nothing.' -f $_.Exception.Message)
  }
  $DownloadedTemp = $true
} else {
  if (-not (Test-Path -LiteralPath $ZipPath -PathType Leaf)) {
    Write-Host ("SourcePath not found: {0}" -f $ZipPath)
    exit 1
  }
  Write-Host ("using local zip: {0} (download skipped)" -f $ZipPath)
}

$zipHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ZipPath).Hash.ToLowerInvariant()
$zipLen = (Get-Item -LiteralPath $ZipPath).Length
$expZipLower = $ExpectedZipSha256.ToLowerInvariant()
Write-Host ("check zip sha256: computed={0} expected={1} size={2} expected-size={3}" -f $zipHash, $expZipLower, $zipLen, $ExpectedZipSize)
if ($zipHash -ne $expZipLower) {
  Write-Host ("zip sha256 mismatch: computed={0} expected={1}; wrote nothing." -f $zipHash, $expZipLower)
  exit 1
}
if ($zipLen -ne $ExpectedZipSize) {
  Write-Host ("zip size mismatch: computed={0} expected={1}; wrote nothing." -f $zipLen, $ExpectedZipSize)
  exit 1
}

# Stage beside the target (same volume) so the final move is an atomic rename.
if (-not (Test-Path -LiteralPath $CacheRoot)) {
  New-Item -ItemType Directory -Path $CacheRoot | Out-Null
}
$stage = Join-Path $CacheRoot ('.stage-' + [System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $stage | Out-Null
$placed = $false
$stagedRoot = $null
try {
  Expand-Archive -LiteralPath $ZipPath -DestinationPath $stage -Force
  $stagedRoot = Join-Path $stage $InnerRoot
  if (-not (Test-Path -LiteralPath $stagedRoot -PathType Container)) {
    Write-Host ('zip missing expected top dir: {0}; wrote nothing.' -f $InnerRoot)
    exit 1
  }
  $stagedExe = Join-Path $stagedRoot 'chrome-headless-shell.exe'
  if (-not (Test-Path -LiteralPath $stagedExe -PathType Leaf)) {
    Write-Host 'zip missing expected entry: chrome-headless-shell.exe; wrote nothing.'
    exit 1
  }

  $want = @{}
  foreach ($e in $Browser.files) { $want[$e.name] = $e }
  $actual = Get-RelFiles $stage
  # Manifest names are zip-relative (include the top dir); actuals are stage-relative: identical sets.
  if ($actual.Count -ne $want.Count) {
    Write-Host ('staged file set mismatch: found={0} expected={1}; wrote nothing.' -f $actual.Count, $want.Count)
    exit 1
  }
  foreach ($a in $actual) {
    $e = $want[$a.Rel]
    if ($null -eq $e) {
      Write-Host ('staged file not in manifest: {0}; wrote nothing.' -f $a.Rel)
      exit 1
    }
    if ([long]$a.Size -ne [long]$e.size) {
      Write-Host ('staged {0}: size mismatch computed={1} expected={2}; wrote nothing.' -f $a.Rel, $a.Size, $e.size)
      exit 1
    }
    $h = (Get-FileHash -Algorithm SHA256 -LiteralPath $a.Full).Hash.ToLowerInvariant()
    if ($h -ne $e.sha256.ToLowerInvariant()) {
      Write-Host ('staged {0}: sha256 mismatch computed={1} expected={2}; wrote nothing.' -f $a.Rel, $h, $e.sha256)
      exit 1
    }
  }
  $exeEntry = $want[$ExeRel]
  $exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $stagedExe).Hash.ToLowerInvariant()
  $exeLen = (Get-Item -LiteralPath $stagedExe).Length
  Write-Host ('check staged {0}: computed={1} expected={2} size={3} expected-size={4}' -f $ExeRel, $exeHash, $exeEntry.sha256, $exeLen, $exeEntry.size)
  Write-Host ('check staged tree: files={0} all match manifest.' -f $actual.Count)

  Move-Item -LiteralPath $stage -Destination $CacheDir
  $placed = $true
  Write-Host ("placed {0}" -f $CacheDir)
} finally {
  if (-not $placed -and (Test-Path -LiteralPath $stage)) {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
  }
  if ($DownloadedTemp -and (Test-Path -LiteralPath $ZipPath)) {
    Remove-Item -LiteralPath $ZipPath -Force -ErrorAction SilentlyContinue
  }
}

if (-not (Test-PinnedBrowserPresent)) {
  Write-Host 'scripts\fetch-chromium.ps1: post-place verification FAILED; refusing.'
  exit 1
}
Write-Host 'chromium provisioned and verified.'
Write-Host ("binary: {0}" -f $ExePath)
