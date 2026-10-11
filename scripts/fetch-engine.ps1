param(
  [string]$SourcePath = ''
)

$ErrorActionPreference = 'Stop'

$UpstreamRepo = 'https://github.com/h4ckf0r0day/obscura'
$Release = 'v0.2.4'
$AssetName = 'obscura-x86_64-windows.zip'
$AssetUrl = 'https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.4/obscura-x86_64-windows.zip'
$ExpectedZipSha256 = '8e5428bf8c101f1439c53a9dcb52d7d09bb22599206b49f6420028d93d01189b'
$ExpectedZipSize = 68520432
$ExpectedObscuraSha256 = '65B9DCADE8B425FEF7C0ED56DFAADBA56C3BA990E572A8B1B0CD8B0226932C0E'
$ExpectedObscuraSize = 78702592
$ExpectedWorkerSha256 = 'D599456C0E08D9333C5B16F7A6307436E6B4A0295FBCE4A605251F2DFCAD93FA'
$ExpectedWorkerSize = 72981504

$RepoRoot = Split-Path -Parent $PSScriptRoot
$BinDir = Join-Path $RepoRoot 'bin'
$ObscuraPath = Join-Path $BinDir 'obscura.exe'
$WorkerPath = Join-Path $BinDir 'obscura-worker.exe'

function Get-Sha256Hex([string]$Path) {
  return (Get-FileHash -Algorithm SHA256 -Path $Path).Hash.ToUpperInvariant()
}

function Test-PinnedBinPresent {
  if (-not (Test-Path -LiteralPath $ObscuraPath -PathType Leaf)) { return $false }
  if (-not (Test-Path -LiteralPath $WorkerPath -PathType Leaf)) { return $false }
  $aHash = Get-Sha256Hex $ObscuraPath
  $aLen = (Get-Item -LiteralPath $ObscuraPath).Length
  $wHash = Get-Sha256Hex $WorkerPath
  $wLen = (Get-Item -LiteralPath $WorkerPath).Length
  Write-Host ("check bin\obscura.exe: computed={0} expected={1} size={2} expected-size={3}" -f $aHash, $ExpectedObscuraSha256, $aLen, $ExpectedObscuraSize)
  Write-Host ("check bin\obscura-worker.exe: computed={0} expected={1} size={2} expected-size={3}" -f $wHash, $ExpectedWorkerSha256, $wLen, $ExpectedWorkerSize)
  return ($aHash -eq $ExpectedObscuraSha256 -and $aLen -eq $ExpectedObscuraSize -and $wHash -eq $ExpectedWorkerSha256 -and $wLen -eq $ExpectedWorkerSize)
}

if (Test-PinnedBinPresent) {
  Write-Host 'already present, verified: bin\obscura.exe and bin\obscura-worker.exe match the pin; nothing changed.'
  exit 0
}

$ZipPath = $SourcePath
$DownloadedTemp = $false
if ([string]::IsNullOrWhiteSpace($ZipPath)) {
  $ZipPath = Join-Path ([System.IO.Path]::GetTempPath()) $AssetName
  Write-Host ("downloading {0} -> {1}" -f $AssetUrl, $ZipPath)
  Invoke-WebRequest -Uri $AssetUrl -OutFile $ZipPath
  $DownloadedTemp = $true
} else {
  if (-not (Test-Path -LiteralPath $ZipPath -PathType Leaf)) {
    Write-Host ("SourcePath not found: {0}" -f $ZipPath)
    exit 1
  }
  Write-Host ("using local zip: {0} (download skipped)" -f $ZipPath)
}

$zipHash = (Get-FileHash -Algorithm SHA256 -Path $ZipPath).Hash.ToLowerInvariant()
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

if (-not (Test-Path -LiteralPath $BinDir)) {
  New-Item -ItemType Directory -Path $BinDir | Out-Null
}

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $stage | Out-Null
try {
  Expand-Archive -LiteralPath $ZipPath -DestinationPath $stage
  $stagedObscura = Join-Path $stage 'obscura.exe'
  $stagedWorker = Join-Path $stage 'obscura-worker.exe'
  if (-not (Test-Path -LiteralPath $stagedObscura -PathType Leaf)) {
    Write-Host 'zip missing expected entry: obscura.exe at root; wrote nothing.'
    exit 1
  }
  if (-not (Test-Path -LiteralPath $stagedWorker -PathType Leaf)) {
    Write-Host 'zip missing expected entry: obscura-worker.exe at root; wrote nothing.'
    exit 1
  }

  $aHash = Get-Sha256Hex $stagedObscura
  $aLen = (Get-Item -LiteralPath $stagedObscura).Length
  $wHash = Get-Sha256Hex $stagedWorker
  $wLen = (Get-Item -LiteralPath $stagedWorker).Length
  Write-Host ("check extracted obscura.exe: computed={0} expected={1} size={2} expected-size={3}" -f $aHash, $ExpectedObscuraSha256, $aLen, $ExpectedObscuraSize)
  Write-Host ("check extracted obscura-worker.exe: computed={0} expected={1} size={2} expected-size={3}" -f $wHash, $ExpectedWorkerSha256, $wLen, $ExpectedWorkerSize)
  if ($aHash -ne $ExpectedObscuraSha256 -or $aLen -ne $ExpectedObscuraSize) {
    Write-Host ("extracted obscura.exe mismatch: computed={0}/{1} expected={2}/{3}; removing what was placed." -f $aHash, $aLen, $ExpectedObscuraSha256, $ExpectedObscuraSize)
    if (Test-Path -LiteralPath $ObscuraPath) { Remove-Item -LiteralPath $ObscuraPath -Force -ErrorAction SilentlyContinue }
    exit 1
  }
  if ($wHash -ne $ExpectedWorkerSha256 -or $wLen -ne $ExpectedWorkerSize) {
    Write-Host ("extracted obscura-worker.exe mismatch: computed={0}/{1} expected={2}/{3}; removing what was placed." -f $wHash, $wLen, $ExpectedWorkerSha256, $ExpectedWorkerSize)
    if (Test-Path -LiteralPath $WorkerPath) { Remove-Item -LiteralPath $WorkerPath -Force -ErrorAction SilentlyContinue }
    exit 1
  }

  Copy-Item -LiteralPath $stagedObscura -Destination $ObscuraPath -Force
  Copy-Item -LiteralPath $stagedWorker -Destination $WorkerPath -Force
  Write-Host ("placed {0}" -f $ObscuraPath)
  Write-Host ("placed {0}" -f $WorkerPath)
} finally {
  Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
  if ($DownloadedTemp -and (Test-Path -LiteralPath $ZipPath)) {
    Remove-Item -LiteralPath $ZipPath -Force -ErrorAction SilentlyContinue
  }
}

$finalA = Get-Sha256Hex $ObscuraPath
$finalW = Get-Sha256Hex $WorkerPath
Write-Host ("verify bin\obscura.exe: computed={0} expected={1}" -f $finalA, $ExpectedObscuraSha256)
Write-Host ("verify bin\obscura-worker.exe: computed={0} expected={1}" -f $finalW, $ExpectedWorkerSha256)
if ($finalA -ne $ExpectedObscuraSha256) {
  Write-Host ("verify mismatch bin\obscura.exe: computed={0} expected={1}; removing mismatched file." -f $finalA, $ExpectedObscuraSha256)
  Remove-Item -LiteralPath $ObscuraPath -Force -ErrorAction SilentlyContinue
}
if ($finalW -ne $ExpectedWorkerSha256) {
  Write-Host ("verify mismatch bin\obscura-worker.exe: computed={0} expected={1}; removing mismatched file." -f $finalW, $ExpectedWorkerSha256)
  Remove-Item -LiteralPath $WorkerPath -Force -ErrorAction SilentlyContinue
}
if ($finalA -ne $ExpectedObscuraSha256 -or $finalW -ne $ExpectedWorkerSha256) {
  exit 1
}
Write-Host 'engine provisioned and verified.'
