$ErrorActionPreference = 'Stop'

$command = if ($args.Length -gt 0) { $args[0] } else { 'install' }
$remaining = if ($args.Length -gt 1) { $args[1..($args.Length - 1)] } else { @() }

$channel = if ($env:SANTI_CHANNEL) { $env:SANTI_CHANNEL } else { 'stable' }
$version = if ($env:SANTI_VERSION) { $env:SANTI_VERSION } else { '' }
$publicUrl = if ($env:SANTI_RELEASES_PUBLIC_URL) { $env:SANTI_RELEASES_PUBLIC_URL } else { 'https://releases.santi.perish.uk' }
$installRoot = if ($env:SANTI_INSTALL_ROOT) { $env:SANTI_INSTALL_ROOT } else { Join-Path $env:LOCALAPPDATA 'santi' }
$localBinDir = if ($env:SANTI_LOCAL_BIN_DIR) { $env:SANTI_LOCAL_BIN_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }
$retain = if ($env:SANTI_RETAIN) { $env:SANTI_RETAIN } else { '' }

for ($i = 0; $i -lt $remaining.Length; $i++) {
    $arg = $remaining[$i]
    switch -Regex ($arg) {
        '^--channel$' { $i++; $channel = $remaining[$i]; continue }
        '^--channel=(.+)$' { $channel = $Matches[1]; continue }
        '^--version$' { $i++; $version = $remaining[$i]; continue }
        '^--version=(.+)$' { $version = $Matches[1]; continue }
        '^--public-url$' { $i++; $publicUrl = $remaining[$i]; continue }
        '^--public-url=(.+)$' { $publicUrl = $Matches[1]; continue }
        '^--install-root$' { $i++; $installRoot = $remaining[$i]; continue }
        '^--install-root=(.+)$' { $installRoot = $Matches[1]; continue }
        '^--bin-dir$' { $i++; $localBinDir = $remaining[$i]; continue }
        '^--bin-dir=(.+)$' { $localBinDir = $Matches[1]; continue }
        '^--retain$' { $retain = 'true'; continue }
        '^--retain=(.+)$' { $retain = $Matches[1]; continue }
        '^(-h|--help|help)$' {
            @'
santi manager

Usage:
  manage.ps1 install [--channel stable|beta] [--version vX.Y.Z] [--retain[=true|false]]
  manage.ps1 uninstall [--version vX.Y.Z]

Environment:
  SANTI_RELEASES_PUBLIC_URL  # default: https://releases.santi.perish.uk
  SANTI_CHANNEL
  SANTI_VERSION
  SANTI_INSTALL_ROOT
  SANTI_LOCAL_BIN_DIR
  SANTI_RETAIN
'@ | Write-Output
            exit 0
        }
        default { throw "unknown argument: $arg" }
    }
}

function Normalize-Version {
    param([string]$Value)
    return "v$($Value.TrimStart('v'))"
}

function Normalize-Bool {
    param([string]$Value)
    switch -Regex ($Value) {
        '^(true|1|yes|y|on)$' { return $true }
        '^(false|0|no|n|off)$' { return $false }
        default { throw "invalid --retain value: $Value" }
    }
}

function Installed-Versions {
    param([string]$Current)
    if (![System.IO.Directory]::Exists($installRoot)) {
        return @()
    }
    return @(Get-ChildItem -LiteralPath $installRoot -Directory | Where-Object { $_.Name -ne $Current } | ForEach-Object { $_.Name })
}

function Should-Retain {
    param([string[]]$OldVersions)
    if ($OldVersions.Length -eq 0) {
        return $true
    }
    if (![string]::IsNullOrWhiteSpace($retain)) {
        return Normalize-Bool $retain
    }
    if ([Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
        $answer = Read-Host 'santi: remove previously installed versions after install? [y/N]'
        if ($answer -match '^(y|yes)$') {
            return $false
        }
        return $true
    }
    [Console]::Error.WriteLine('santi: preserving previous versions; pass --retain=false to prune after install')
    return $true
}

function Install-Santi {
    $resolvedPublicUrl = $publicUrl.TrimEnd('/')
    $resolvedVersion = $version
    if ([string]::IsNullOrWhiteSpace($resolvedVersion)) {
        $metadataUrl = "$resolvedPublicUrl/$channel/latest/metadata.json"
        $metadata = Invoke-RestMethod -Uri $metadataUrl
        $resolvedVersion = $metadata.releaseVersion
        if ([string]::IsNullOrWhiteSpace($resolvedVersion)) {
            throw 'failed to resolve latest santi version'
        }
    }
    $resolvedVersion = Normalize-Version $resolvedVersion
    $oldVersions = Installed-Versions $resolvedVersion
    $retainOld = Should-Retain $oldVersions

    $archive = 'santi-x86_64-pc-windows-msvc.zip'
    $tmpdir = Join-Path ([System.IO.Path]::GetTempPath()) ("santi-" + [System.Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmpdir | Out-Null
    try {
        $archivePath = Join-Path $tmpdir $archive
        Invoke-WebRequest -Uri "$resolvedPublicUrl/$channel/versions/$resolvedVersion/$archive" -OutFile $archivePath
        $versionRoot = Join-Path $installRoot $resolvedVersion
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $versionRoot
        New-Item -ItemType Directory -Force -Path $versionRoot | Out-Null
        Expand-Archive -LiteralPath $archivePath -DestinationPath $versionRoot -Force
        New-Item -ItemType Directory -Force -Path $localBinDir | Out-Null
        Copy-Item -Force (Join-Path $versionRoot 'santi.exe') (Join-Path $localBinDir 'santi.exe')
        & (Join-Path $localBinDir 'santi.exe') --version

        if (!$retainOld) {
            foreach ($oldVersion in $oldVersions) {
                Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $installRoot $oldVersion)
                Write-Output "removed old santi $oldVersion from $installRoot"
            }
        }

        Write-Output "installed santi to $(Join-Path $localBinDir 'santi.exe')"
    }
    finally {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmpdir
    }
}

function Remove-EmptyDir {
    param([string]$Path)
    if ([System.IO.Directory]::Exists($Path)) {
        try {
            Remove-Item -Force -ErrorAction Stop $Path
        }
        catch [System.IO.IOException] {}
    }
}

function Installed-Version {
    $binPath = Join-Path $localBinDir 'santi.exe'
    if (![System.IO.File]::Exists($binPath)) {
        return ''
    }
    try {
        $output = & $binPath --version
        if ($output -match 'v?([0-9]+\.[0-9]+\.[0-9]+(?:[-.][A-Za-z0-9]+)*)') {
            return "v$($Matches[1].TrimStart('v'))"
        }
    }
    catch {}
    return ''
}

function Uninstall-Santi {
    $binPath = Join-Path $localBinDir 'santi.exe'
    if (![string]::IsNullOrWhiteSpace($version)) {
        $normalizedVersion = Normalize-Version $version
        if ((Installed-Version) -eq $normalizedVersion) {
            Remove-Item -Force -ErrorAction SilentlyContinue $binPath
            Write-Output "removed $binPath"
        }
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $installRoot $normalizedVersion)
        Remove-EmptyDir $installRoot
        Write-Output "removed santi $normalizedVersion from $installRoot"
        return
    }

    Remove-Item -Force -ErrorAction SilentlyContinue $binPath
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $installRoot
    Remove-EmptyDir $localBinDir
    Write-Output "removed santi from $installRoot and $binPath"
}

switch ($command) {
    'install' { Install-Santi }
    'uninstall' { Uninstall-Santi }
    default { throw "unknown command: $command" }
}
