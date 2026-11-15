<#
.SYNOPSIS
  Verifies Windows packaging parity against `hybrid-player/electron-builder.json`
  and stages `win-unpacked` resources (`mpv`, `ffmpeg`, `assets`) for `cargo-packager` / NSIS.

.DESCRIPTION
  Checks:
  1. Metadata parity (`appId`, `productName`, `version`, `nsis.*`) between
     `hybrid-player/electron-builder.json`, `hybrid-player/package.json`,
     `packaging/Packager.toml`, `packaging/bundle-manifest.json`, and `packaging/nsis/installer.nsh`.
  2. All 10 media file associations (`mp4, mkv, webm, avi, mov, mp3, flac, wav, ogg, m4a`).
  3. Presence and minimum byte sizes (`> 1024 B` matching `hybrid_core::binary_resolver::is_usable_file`)
     for `libmpv-2.dll`, `d3dcompiler_43.dll`, `ffmpeg.exe`, `yt-dlp.exe`, `mpv.exe`,
     plus all 6 fonts and 7 icons.
  4. Available Windows packaging tools (`cargo-packager`, `makensis.exe`, `wix`, `rcedit-x64.exe`, `signtool`).
  5. Optional `-Stage`: stages files into `-StagingDir` mirroring `resources/{mpv,ffmpeg,assets}`
     and verifies SHA-256 checksums of every staged file.
#>
[CmdletBinding()]
param(
    [switch]$Stage,
    [switch]$AllowMissingAppExe,
    [string]$StagingDir = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($StagingDir)) {
    $StagingDir = Join-Path $PSScriptRoot "staging\win-unpacked"
}

$EbPath = Join-Path $RepoRoot "hybrid-player\electron-builder.json"
$PkgJsonPath = Join-Path $RepoRoot "hybrid-player\package.json"
$ManifestPath = Join-Path $PSScriptRoot "bundle-manifest.json"
$PackagerTomlPath = Join-Path $PSScriptRoot "Packager.toml"
$NsisHookPath = Join-Path $PSScriptRoot "nsis\installer.nsh"

foreach ($req in @($EbPath, $PkgJsonPath, $ManifestPath, $PackagerTomlPath, $NsisHookPath)) {
    if (-not (Test-Path $req)) {
        throw "Required packaging/parity file missing: $req"
    }
}

$eb = Get-Content $EbPath -Raw | ConvertFrom-Json
$pkg = Get-Content $PkgJsonPath -Raw | ConvertFrom-Json
$manifest = Get-Content $ManifestPath -Raw | ConvertFrom-Json
$toml = Get-Content $PackagerTomlPath -Raw
$nsh = Get-Content $NsisHookPath -Raw

$failures = [System.Collections.Generic.List[string]]::new()

function Assert-Check {
    param([bool]$Condition, [string]$Message)
    if ($Condition) {
        Write-Host "[PASS] $Message"
    } else {
        Write-Host "[FAIL] $Message" -ForegroundColor Red
        $failures.Add($Message) | Out-Null
    }
}

# 1. Core metadata parity
Assert-Check ($eb.appId -eq "com.hybridplayer.app") "electron-builder.json appId == com.hybridplayer.app"
Assert-Check ($manifest.appId -eq $eb.appId) "bundle-manifest.json appId matches electron-builder.json"
Assert-Check ($toml -match 'identifier\s*=\s*"com\.hybridplayer\.app"') "Packager.toml identifier == com.hybridplayer.app"
Assert-Check ($nsh -match 'HYBRID_APP_ID\s+"com\.hybridplayer\.app"') "installer.nsh HYBRID_APP_ID == com.hybridplayer.app"

Assert-Check ($manifest.productName -eq $eb.productName) "productName matches ($($eb.productName))"
Assert-Check ($toml -match 'product-name\s*=\s*"Hybrid Player"') "Packager.toml product-name == Hybrid Player"
Assert-Check ($manifest.version -eq $pkg.version) "bundle-manifest.json version matches package.json ($($pkg.version))"
Assert-Check ($toml -match ('version\s*=\s*"' + [regex]::Escape($pkg.version) + '"')) "Packager.toml version == $($pkg.version)"

# 2. NSIS flags parity
Assert-Check ($manifest.nsis.oneClick -eq $eb.nsis.oneClick) "NSIS oneClick == $($eb.nsis.oneClick)"
Assert-Check ($manifest.nsis.allowToChangeInstallationDirectory -eq $eb.nsis.allowToChangeInstallationDirectory) "NSIS allowToChangeInstallationDirectory == $($eb.nsis.allowToChangeInstallationDirectory)"
Assert-Check ($manifest.nsis.createDesktopShortcut -eq $eb.nsis.createDesktopShortcut) "NSIS createDesktopShortcut == $($eb.nsis.createDesktopShortcut)"
Assert-Check ($manifest.nsis.createStartMenuShortcut -eq $eb.nsis.createStartMenuShortcut) "NSIS createStartMenuShortcut == $($eb.nsis.createStartMenuShortcut)"

# 3. File associations parity (all 10 extensions across all 4 files)
$ebExts = @($eb.fileAssociations | ForEach-Object { $_.ext })
$mfExts = @($manifest.fileAssociations | ForEach-Object { $_.ext })
Assert-Check ($ebExts.Count -eq 10) "electron-builder.json defines 10 file associations ($($ebExts -join ', '))"
Assert-Check (($ebExts -join ',') -eq ($mfExts -join ',')) "bundle-manifest.json fileAssociations match electron-builder.json 1:1"

foreach ($assoc in $eb.fileAssociations) {
    $ext = $assoc.ext
    $name = $assoc.name
    $inToml = $toml -match ('extensions\s*=\s*\["' + [regex]::Escape($ext) + '"\]')
    $inNshReg = $nsh -match ('HYBRID_REGISTER_EXT\s+"' + [regex]::Escape($ext) + '"\s+"' + [regex]::Escape($name) + '"')
    $inNshUnreg = $nsh -match ('HYBRID_UNREGISTER_EXT\s+"' + [regex]::Escape($ext) + '"')
    Assert-Check ($inToml -and $inNshReg -and $inNshUnreg) "Association .$ext ($name) present in Packager.toml + installer.nsh register/unregister"
}

# 4. Required DLLs / EXEs / Assets check
foreach ($bin in $manifest.binaries) {
    $srcPath = Join-Path $RepoRoot $bin.source
    $exists = Test-Path $srcPath
    if ($exists) {
        $len = (Get-Item $srcPath).Length
        $sizeOk = $len -ge [int64]$bin.minBytes
        Assert-Check $sizeOk "Binary $($bin.source) exists ($len bytes >= $($bin.minBytes))"
    } elseif ($bin.required) {
        Assert-Check $false "Required binary missing: $($bin.source)"
    } else {
        Write-Host "[SKIP] Optional binary absent: $($bin.source)"
    }
}

foreach ($assetRel in $manifest.assets) {
    $assetPath = Join-Path $RepoRoot $assetRel
    $exists = Test-Path $assetPath
    $len = if ($exists) { (Get-Item $assetPath).Length } else { 0 }
    Assert-Check ($exists -and $len -gt 0) "Asset $assetRel present ($len bytes)"
}

# Also check Packager.toml binary name parity and `{ src, target = "resources/..." }` resource mappings
Assert-Check ($toml -match 'path\s*=\s*"staging/win-unpacked/Hybrid Player\.exe"') "Packager.toml main binary path aligns with Hybrid Player.exe"
Assert-Check ($nsh -match '\$INSTDIR\\Hybrid Player\.exe') "installer.nsh launches/registers Hybrid Player.exe"
foreach ($targetRel in @(
    "resources/mpv/libmpv-2.dll",
    "resources/mpv/d3dcompiler_43.dll",
    "resources/mpv/yt-dlp.exe",
    "resources/ffmpeg/ffmpeg.exe",
    "resources/assets/fonts",
    "resources/assets/icons"
)) {
    $hasMapping = $toml -match ('target\s*=\s*"' + [regex]::Escape($targetRel) + '"')
    Assert-Check $hasMapping "Packager.toml maps resource target '$targetRel'"
}
Assert-Check ($toml -match [regex]::Escape("icon.ico")) "Packager.toml includes icon 'icon.ico'"

# 5. App binary check (optional pre-Phase 5)
$appExePath = Join-Path $RepoRoot $manifest.sourceBinary
$hasAppExe = Test-Path $appExePath
if ($hasAppExe) {
    Write-Host "[PASS] Built desktop binary present: $appExePath"
} elseif ($AllowMissingAppExe) {
    Write-Host "[INFO] Desktop binary not yet built (expected pre-Phase 5): $appExePath"
} else {
    Assert-Check $false "Desktop binary missing (build hybrid-app first or pass -AllowMissingAppExe): $appExePath"
}

# 6. Tool availability report
Write-Host ""
Write-Host "--- Windows Packaging Tool Probe ---"
$ebNsis = Get-ChildItem "$env:LOCALAPPDATA\electron-builder\Cache\nsis" -Recurse -Filter "makensis.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
$ebRcedit = Get-ChildItem "$env:LOCALAPPDATA\electron-builder\Cache\winCodeSign" -Recurse -Filter "rcedit-x64.exe" -ErrorAction SilentlyContinue | Select-Object -First 1

$toolProbe = @(
    @{ Name = "cargo-packager"; Path = (Get-Command "cargo-packager" -ErrorAction SilentlyContinue).Source },
    @{ Name = "makensis (PATH)"; Path = (Get-Command "makensis" -ErrorAction SilentlyContinue).Source },
    @{ Name = "makensis (electron-builder cache)"; Path = if ($ebNsis) { $ebNsis.FullName } else { $null } },
    @{ Name = "rcedit-x64 (electron-builder cache)"; Path = if ($ebRcedit) { $ebRcedit.FullName } else { $null } },
    @{ Name = "wix"; Path = (Get-Command "wix" -ErrorAction SilentlyContinue).Source },
    @{ Name = "candle"; Path = (Get-Command "candle" -ErrorAction SilentlyContinue).Source },
    @{ Name = "light"; Path = (Get-Command "light" -ErrorAction SilentlyContinue).Source },
    @{ Name = "signtool"; Path = (Get-Command "signtool" -ErrorAction SilentlyContinue).Source }
)
foreach ($t in $toolProbe) {
    $status = if ($t.Path) { $t.Path } else { "<not installed>" }
    Write-Host ("  {0,-36} : {1}" -f $t.Name, $status)
}

# 7. Optional Staging + SHA-256 parity verification
if ($Stage) {
    Write-Host ""
    Write-Host "--- Staging win-unpacked layout to $StagingDir ---"
    if (Test-Path $StagingDir) {
        Remove-Item $StagingDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $StagingDir -Force | Out-Null

    if ($hasAppExe) {
        Copy-Item $appExePath (Join-Path $StagingDir $manifest.binaryName) -Force
    }

    foreach ($bin in $manifest.binaries) {
        $srcPath = Join-Path $RepoRoot $bin.source
        if (-not (Test-Path $srcPath)) { continue }
        $dstPath = Join-Path $StagingDir $bin.dest
        New-Item -ItemType Directory -Path (Split-Path $dstPath -Parent) -Force | Out-Null
        Copy-Item $srcPath $dstPath -Force
        $srcHash = (Get-FileHash $srcPath -Algorithm SHA256).Hash
        $dstHash = (Get-FileHash $dstPath -Algorithm SHA256).Hash
        Assert-Check ($srcHash -eq $dstHash) "Staged $($bin.dest) SHA-256 verified"
    }

    foreach ($assetRel in $manifest.assets) {
        $srcPath = Join-Path $RepoRoot $assetRel
        $subRel = $assetRel -replace '^hybrid-player[/\\]', 'resources/'
        $dstPath = Join-Path $StagingDir $subRel
        New-Item -ItemType Directory -Path (Split-Path $dstPath -Parent) -Force | Out-Null
        Copy-Item $srcPath $dstPath -Force
        $srcHash = (Get-FileHash $srcPath -Algorithm SHA256).Hash
        $dstHash = (Get-FileHash $dstPath -Algorithm SHA256).Hash
        Assert-Check ($srcHash -eq $dstHash) "Staged $subRel SHA-256 verified"
    }

    # Verify packaged resource folder tree matches `hybrid_core::binary_resolver` expectations
    Assert-Check (Test-Path (Join-Path $StagingDir "resources\mpv\libmpv-2.dll")) "Staged layout contains resources/mpv/libmpv-2.dll"
    Assert-Check (Test-Path (Join-Path $StagingDir "resources\mpv\d3dcompiler_43.dll")) "Staged layout contains resources/mpv/d3dcompiler_43.dll"
    Assert-Check (Test-Path (Join-Path $StagingDir "resources\mpv\yt-dlp.exe")) "Staged layout contains resources/mpv/yt-dlp.exe"
    Assert-Check (Test-Path (Join-Path $StagingDir "resources\ffmpeg\ffmpeg.exe")) "Staged layout contains resources/ffmpeg/ffmpeg.exe"
    Assert-Check ((Get-ChildItem (Join-Path $StagingDir "resources\assets\fonts") -File).Count -eq 6) "Staged layout contains all 6 font files"
    Assert-Check ((Get-ChildItem (Join-Path $StagingDir "resources\assets\icons") -File).Count -eq 7) "Staged layout contains all 7 icon files"
}

Write-Host ""
if ($failures.Count -gt 0) {
    Write-Host "Packaging verification FAILED ($($failures.Count) failures)." -ForegroundColor Red
    exit 1
}

Write-Host "Packaging verification PASSED (0 failures)." -ForegroundColor Green
exit 0
