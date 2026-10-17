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

  Update path: -Check resolves the newest Stable CfT build from
  last-known-good-versions.json, compares it with the pin, prints both plus
  whether an update is available, and downloads nothing. -Update resolves the
  newest Stable (or an explicit -Version override), recomputes the entire
  manifest from the real downloaded files when it differs from the pin, and
  is a no-op (no download, no rewrite) when it equals the pin.
#>
param(
  [string]$SourcePath = '',
  [switch]$Force,
  [switch]$Check,
  [switch]$Update,
  [string]$Version = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Pin (keep in lockstep with scripts/chromium-manifest.json).
$PinnedVersion = '155.0.8059.84'
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
$GoodVersionsUrl = 'https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json'

function Get-StableCfTVersion {
  try {
    $g = Invoke-RestMethod -Uri $GoodVersionsUrl
  } catch {
    Write-Host ('scripts\fetch-chromium.ps1: failed to resolve Stable CfT version from {0}: {1}; wrote nothing.' -f $GoodVersionsUrl, $_.Exception.Message)
    exit 1
  }
  $v = $g.channels.Stable.version
  if ([string]::IsNullOrWhiteSpace($v)) {
    Write-Host ('scripts\fetch-chromium.ps1: Stable channel version missing in {0}; wrote nothing.' -f $GoodVersionsUrl)
    exit 1
  }
  return $v
}

function Get-CfTAssetUrl([string]$Ver) {
  return ('https://storage.googleapis.com/chrome-for-testing-public/{0}/win64/chrome-headless-shell-win64.zip' -f $Ver)
}

if ($Check -and $Update) {
  Write-Host 'scripts\fetch-chromium.ps1: -Check and -Update are mutually exclusive; wrote nothing.'
  exit 1
}
if ($Version -ne '' -and -not $Check -and -not $Update) {
  Write-Host 'scripts\fetch-chromium.ps1: -Version requires -Check or -Update; wrote nothing.'
  exit 1
}

if ($Check) {
  $target = if ($Version -ne '') { $Version } else { Get-StableCfTVersion }
  $src = if ($Version -ne '') { 'explicit -Version override' } else { $GoodVersionsUrl }
  Write-Host ('pinned version: {0}' -f $PinnedVersion)
  Write-Host ('latest Stable version: {0} (resolved from {1})' -f $target, $src)
  if ($target -eq $PinnedVersion) {
    Write-Host 'update available: no (pinned is latest)'
  } else {
    Write-Host ('update available: yes ({0} -> {1})' -f $PinnedVersion, $target)
    Write-Host 'run with -Update to download, verify, and recompute scripts/chromium-manifest.json.'
  }
  Write-Host 'check complete: downloaded nothing.'
  exit 0
}

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
if ($Browser.version -ne $PinnedVersion -or $Browser.asset.url -ne $AssetUrl -or
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
$CacheDir = Join-Path $CacheRoot ("{0}-{1}" -f $PinnedVersion, $ZipShort8)
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

if ($Update) {
  $target = if ($Version -ne '') { $Version } else { Get-StableCfTVersion }
  $src = if ($Version -ne '') { 'explicit -Version override' } else { $GoodVersionsUrl }
  Write-Host ('pinned version: {0}' -f $PinnedVersion)
  Write-Host ('target version: {0} (resolved from {1})' -f $target, $src)
  if ($target -eq $PinnedVersion) {
    Write-Host ('already pinned at {0}; no update; manifest untouched.' -f $PinnedVersion)
    exit 0
  }
  $oldVersion = $PinnedVersion
  $oldFileCount = [long]$Browser.fileCount
  $oldTotalSize = [long]$Browser.totalSize
  $newVersion = $target
  $newAssetUrl = Get-CfTAssetUrl $newVersion
  Write-Host ('update available: yes ({0} -> {1})' -f $oldVersion, $newVersion)
  Write-Host ("downloading {0}" -f $newAssetUrl)

  $tmpZip = $SourcePath
  $downloadedTemp = $false
  if ([string]::IsNullOrWhiteSpace($tmpZip)) {
    $tmpZip = Join-Path ([System.IO.Path]::GetTempPath()) ('chrome-headless-shell-{0}-{1}.zip' -f $newVersion, [System.IO.Path]::GetRandomFileName())
    try {
      Invoke-WebRequest -Uri $newAssetUrl -OutFile $tmpZip
    } catch {
      Write-Host ('scripts\fetch-chromium.ps1: update download failed: {0}; wrote nothing.' -f $_.Exception.Message)
      exit 1
    }
    $downloadedTemp = $true
  } else {
    if (-not (Test-Path -LiteralPath $tmpZip -PathType Leaf)) {
      Write-Host ("SourcePath not found: {0}" -f $tmpZip)
      exit 1
    }
    Write-Host ("using local zip: {0} (download skipped)" -f $tmpZip)
  }

  $newZipHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmpZip).Hash.ToLowerInvariant()
  $newZipSize = (Get-Item -LiteralPath $tmpZip).Length
  $newShort8 = $newZipHash.Substring(0, 8)
  Write-Host ('update zip sha256: {0} size={1}' -f $newZipHash, $newZipSize)
  $newCacheDir = Join-Path $CacheRoot ('{0}-{1}' -f $newVersion, $newShort8)

  if (-not (Test-Path -LiteralPath $CacheRoot)) {
    New-Item -ItemType Directory -Path $CacheRoot | Out-Null
  }
  $updStage = Join-Path $CacheRoot ('.stage-' + [System.IO.Path]::GetRandomFileName())
  New-Item -ItemType Directory -Path $updStage | Out-Null
  $updPlaced = $false
  try {
    Expand-Archive -LiteralPath $tmpZip -DestinationPath $updStage -Force
    $updRoot = Join-Path $updStage $InnerRoot
    if (-not (Test-Path -LiteralPath $updRoot -PathType Container)) {
      Write-Host ('update zip missing expected top dir: {0}; wrote nothing.' -f $InnerRoot)
      exit 1
    }
    $updExe = Join-Path $updRoot 'chrome-headless-shell.exe'
    if (-not (Test-Path -LiteralPath $updExe -PathType Leaf)) {
      Write-Host 'update zip missing expected entry: chrome-headless-shell.exe; wrote nothing.'
      exit 1
    }
    $updFiles = Get-RelFiles $updStage | Sort-Object Rel
    if ($updFiles.Count -eq 0) {
      Write-Host 'update zip contains no files; wrote nothing.'
      exit 1
    }
    $fileEntries = @()
    $totalSize = [long]0
    foreach ($f in $updFiles) {
      $h = (Get-FileHash -Algorithm SHA256 -LiteralPath $f.Full).Hash.ToLowerInvariant()
      $totalSize += [long]$f.Size
      $fileEntries += [ordered]@{ name = $f.Rel; sha256 = $h; size = [long]$f.Size }
    }
    $fileCount = $updFiles.Count
    $sizeDelta = $totalSize - $oldTotalSize
    $today = (Get-Date -Format 'yyyy-MM-dd')
    $resolvedFrom = if ($Version -ne '') {
      ('explicit -Version {0} (CfT assetExists check {1}; a channel name is NOT a pin, the resolved version string is)' -f $newVersion, $today)
    } else {
      ('{0} (Stable channel pin, read {1}; a channel name is NOT a pin, the resolved version string is)' -f $GoodVersionsUrl, $today)
    }
    $comment = ('Pinned Chrome-for-Testing chrome-headless-shell manifest for gladius (win64 only). Hashes computed BY US on {0} from the real downloaded archive: downloaded {1} via Invoke-WebRequest, hashed the zip with Get-FileHash SHA256, extracted with Expand-Archive, then hashed every extracted file with SHA256 and recorded sizes. The upstream CfT JSON endpoints carry NO sha256 field, so this file is the sole integrity root. scripts/fetch-chromium.ps1 verifies the zip AND every listed file (exact set, no extras) before provisioning, and re-verifies on reuse. Keep this file, the script constants, and the pinned version in lockstep.' -f $today, $newAssetUrl)
    $newManifest = [ordered]@{
      _comment = $comment
      browser = [ordered]@{
        project = 'Chrome for Testing: chrome-headless-shell (headless-only //content wrapper)'
        version = $newVersion
        platform = $Platform
        resolvedFrom = $resolvedFrom
        asset = [ordered]@{
          name = $AssetName
          url = $newAssetUrl
          sha256 = $newZipHash
          size = [long]$newZipSize
        }
        root = $InnerRoot
        fileCount = [long]$fileCount
        totalSize = [long]$totalSize
        files = $fileEntries
      }
    }
    $json = $newManifest | ConvertTo-Json -Depth 8
    $json = $json -replace "`r`n", "`n"
    $json = $json -replace "`n", "`r`n"
    [System.IO.File]::WriteAllText($ManifestPath, $json, [System.Text.UTF8Encoding]::new($false))
    Write-Host ('rewrote {0}' -f $ManifestPath)

    $selfPath = $PSCommandPath
    if ([string]::IsNullOrWhiteSpace($selfPath)) { $selfPath = Join-Path $PSScriptRoot 'fetch-chromium.ps1' }
    $selfText = [System.IO.File]::ReadAllText($selfPath)
    $selfText = $selfText -replace ([regex]::Escape(('$PinnedVersion = ''{0}''' -f $oldVersion))), ('$PinnedVersion = ''{0}''' -f $newVersion)
    $selfText = $selfText -replace [regex]::Escape($AssetUrl), $newAssetUrl
    $selfText = $selfText -replace [regex]::Escape($ExpectedZipSha256), $newZipHash
    $selfText = $selfText -replace ([regex]::Escape(('$ExpectedZipSize = {0}' -f $ExpectedZipSize))), ('$ExpectedZipSize = {0}' -f $newZipSize)
    [System.IO.File]::WriteAllText($selfPath, $selfText, [System.Text.UTF8Encoding]::new($false))
    Write-Host 'updated pin constants in scripts/fetch-chromium.ps1'

    if (Test-Path -LiteralPath $newCacheDir) {
      $already = Get-RelFiles $newCacheDir | Sort-Object Rel
      $match = ($already.Count -eq $fileCount)
      if ($match) {
        $wantNew = @{}
        foreach ($e in $fileEntries) { $wantNew[$e.name] = $e }
        foreach ($a in $already) {
          $e = $wantNew[$a.Rel]
          if ($null -eq $e -or [long]$a.Size -ne [long]$e.size) { $match = $false; break }
          $hh = (Get-FileHash -Algorithm SHA256 -LiteralPath $a.Full).Hash.ToLowerInvariant()
          if ($hh -ne $e.sha256.ToLowerInvariant()) { $match = $false; break }
        }
      }
      if ($match) {
        Write-Host ('already present, verified: {0} matches the new pin; kept.' -f $newCacheDir)
        $updPlaced = $true
        Remove-Item -LiteralPath $updStage -Recurse -Force -ErrorAction SilentlyContinue
      } elseif (-not $Force) {
        Write-Host ('scripts\fetch-chromium.ps1: REFUSING: existing cache at {0} does not match the new pin; wrote manifest+script but did not replace cache. Delete that directory, or re-run with -Force; wrote nothing to cache.' -f $newCacheDir)
        Remove-Item -LiteralPath $updStage -Recurse -Force -ErrorAction SilentlyContinue
        if ($downloadedTemp -and (Test-Path -LiteralPath $tmpZip)) { Remove-Item -LiteralPath $tmpZip -Force -ErrorAction SilentlyContinue }
        exit 1
      } else {
        Write-Host ('-Force: removing mismatched cache at {0}' -f $newCacheDir)
        Remove-Item -LiteralPath $newCacheDir -Recurse -Force
      }
    }
    if (-not $updPlaced) {
      Move-Item -LiteralPath $updStage -Destination $newCacheDir
      $updPlaced = $true
      Write-Host ("placed {0}" -f $newCacheDir)
    }

    $verifyWant = @{}
    foreach ($e in $fileEntries) { $verifyWant[$e.name] = $e }
    $placedFiles = Get-RelFiles $newCacheDir | Sort-Object Rel
    $bad = 0
    foreach ($a in $placedFiles) {
      $e = $verifyWant[$a.Rel]
      if ($null -eq $e) { Write-Host ('verify extra file not in manifest: {0}' -f $a.Rel); $bad++; continue }
      if ([long]$a.Size -ne [long]$e.size) { Write-Host ('verify {0}: size mismatch' -f $a.Rel); $bad++; continue }
      $hh = (Get-FileHash -Algorithm SHA256 -LiteralPath $a.Full).Hash.ToLowerInvariant()
      if ($hh -ne $e.sha256.ToLowerInvariant()) { Write-Host ('verify {0}: sha256 mismatch' -f $a.Rel); $bad++ }
    }
    $newExePath = Join-Path $newCacheDir $ExeRel.Replace('/', '\')
    $newExeEntry = $verifyWant[$ExeRel]
    $newExeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $newExePath).Hash.ToLowerInvariant()
    $newExeLen = (Get-Item -LiteralPath $newExePath).Length
    Write-Host ('check {0}: computed={1} expected={2} size={3} expected-size={4}' -f $ExeRel, $newExeHash, $newExeEntry.sha256, $newExeLen, $newExeEntry.size)
    Write-Host ('check tree: files={0} bytes={1} mismatches={2}' -f $placedFiles.Count, $totalSize, $bad)
    if ($bad -ne 0 -or $placedFiles.Count -ne $fileCount) {
      Write-Host 'scripts\fetch-chromium.ps1: post-update verification FAILED; refusing.'
      exit 1
    }
    Write-Host ('browser bump: {0} -> {1}' -f $oldVersion, $newVersion)
    Write-Host ('file count: {0} -> {1}' -f $oldFileCount, $fileCount)
    $sign = if ($sizeDelta -ge 0) { '+' } else { '' }
    Write-Host ('total size: {0} -> {1} ({2}{3} bytes)' -f $oldTotalSize, $totalSize, $sign, $sizeDelta)
    Write-Host ('zip sha256: {0} size={1}' -f $newZipHash, $newZipSize)
    Write-Host 'chromium manifest updated and verified.'
  } finally {
    if (-not $updPlaced -and (Test-Path -LiteralPath $updStage)) {
      Remove-Item -LiteralPath $updStage -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($downloadedTemp -and (Test-Path -LiteralPath $tmpZip)) {
      Remove-Item -LiteralPath $tmpZip -Force -ErrorAction SilentlyContinue
    }
  }
  exit 0
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
