<#
.SYNOPSIS
Installs branch-tally from a GitHub release (Windows).

.DESCRIPTION
Downloads the release archive for this machine's OS architecture (x64 or arm64), verifies it against
the release's SHA256SUMS, and installs branch-tally.exe. A checksum mismatch, or an archive missing
from SHA256SUMS, is a hard failure and nothing is installed. Never edits the user PATH: if the install
directory is not on PATH, the exact command to add it is printed.

One line:  irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex

`irm | iex` cannot pass parameters, so each has an environment variable. A parameter overrides it.

  -Version             BRANCH_TALLY_INSTALL_VERSION             vX.Y.Z, or latest (the default)
  -To                  BRANCH_TALLY_INSTALL_DIR                 default: %USERPROFILE%\.local\bin
                       BRANCH_TALLY_INSTALL_BASE_URL            release mirror (default: the GitHub
                                                                releases URL of the repository)

.PARAMETER Version
Release to install, such as v0.4.0. Default: latest.

.PARAMETER To
Directory to install into. Default: %USERPROFILE%\.local\bin.

#>
param(
  [string]$Version,
  [string]$To
)

function Install-BranchTally {
  param(
    [string]$Version,
    [string]$To
  )

  Set-StrictMode -Version Latest
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'

  $repo = 'IAmSkweetis/branch-tally'

  if ([string]::IsNullOrEmpty($Version)) { $Version = $env:BRANCH_TALLY_INSTALL_VERSION }
  if ([string]::IsNullOrEmpty($Version)) { $Version = 'latest' }
  if ([string]::IsNullOrEmpty($To)) { $To = $env:BRANCH_TALLY_INSTALL_DIR }
  $base = $env:BRANCH_TALLY_INSTALL_BASE_URL
  if ([string]::IsNullOrEmpty($base)) { $base = "https://github.com/$repo/releases" }
  $base = $base.TrimEnd('/')

  if ($Version -ne 'latest') {
    if (-not $Version.StartsWith('v')) { $Version = "v$Version" }
    if ($Version -notmatch '^v\d+\.\d+\.\d+$') {
      throw "version '$Version' is not of the form vMAJOR.MINOR.PATCH"
    }
  }

  if ([string]::IsNullOrEmpty($To)) {
    $profileDir = $env:USERPROFILE
    if ([string]::IsNullOrEmpty($profileDir)) { $profileDir = $HOME }
    if ([string]::IsNullOrEmpty($profileDir)) { throw 'USERPROFILE is not set; pass -To <dir>' }
    $To = Join-Path (Join-Path $profileDir '.local') 'bin'
  }

  # Platform: Windows only, and the OS architecture, not this process's (a 32-bit or emulated
  # PowerShell on an arm64 machine still gets the arm64 build).
  $isWin = $true
  $isWinVar = Get-Variable -Name IsWindows -ErrorAction SilentlyContinue
  if ($null -ne $isWinVar) { $isWin = [bool]$isWinVar.Value }
  if (-not $isWin) { throw 'this installer is for Windows; on Linux and macOS use install.sh' }

  $arch = $null
  try {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
  } catch {
    $arch = $null
  }
  if ([string]::IsNullOrEmpty($arch)) {
    $arch = $env:PROCESSOR_ARCHITEW6432
    if ([string]::IsNullOrEmpty($arch)) { $arch = $env:PROCESSOR_ARCHITECTURE }
  }
  switch -Regex ($arch) {
    '^(X64|AMD64)$' { $target = 'x86_64-pc-windows-msvc' }
    '^(Arm64|ARM64)$' { $target = 'aarch64-pc-windows-msvc' }
    default { throw "unsupported architecture '$arch' on Windows" }
  }

  try {
    [Net.ServicePointManager]::SecurityProtocol =
      [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  } catch {
    # Newer runtimes negotiate TLS themselves.
    $null = $_
  }

  function Get-Remote {
    param([string]$Url, [string]$OutFile)
    if ($Url.StartsWith('file://')) {
      # Invoke-WebRequest has no file:// support. This is the test and mirror seam.
      Copy-Item -LiteralPath ([Uri]$Url).LocalPath -Destination $OutFile
    } else {
      Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
    }
  }

  function Get-ExpectedHash {
    param([string]$SumsFile, [string]$Name)
    foreach ($line in [System.IO.File]::ReadAllLines($SumsFile)) {
      $parts = $line.Trim() -split '\s+'
      if ($parts.Count -ge 2) {
        $n = $parts[1].TrimStart('*')
        if ($n -ceq $Name) { return $parts[0].ToLowerInvariant() }
      }
    }
    return $null
  }

  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('branch-tally-install-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $tmp | Out-Null
  $staged = $null
  try {
    # Resolve "latest" to a tag without the GitHub API (no rate limits): the archive names embed the
    # version, so read this target's name from latest/download/SHA256SUMS. Everything after comes
    # from download/<tag>/, never latest/, so a release landing mid-install cannot mix versions; the
    # checksum is still verified before anything is installed.
    if ($Version -eq 'latest') {
      $latestSums = Join-Path $tmp 'latest.sums'
      try {
        Get-Remote "$base/latest/download/SHA256SUMS" $latestSums
      } catch {
        throw "could not download $base/latest/download/SHA256SUMS"
      }
      $tag = $null
      $pattern = '^branch-tally-(v\d+\.\d+\.\d+)-' + [regex]::Escape($target) + '\.zip$'
      foreach ($line in [System.IO.File]::ReadAllLines($latestSums)) {
        $parts = $line.Trim() -split '\s+'
        if ($parts.Count -ge 2 -and $parts[1].TrimStart('*') -match $pattern) {
          $tag = $Matches[1]
          break
        }
      }
      if (-not $tag) { throw "the latest release has no build for $target" }
    } else {
      $tag = $Version
    }

    $archive = "branch-tally-$tag-$target.zip"
    $url = "$base/download/$tag"
    Write-Output "Installing branch-tally $tag for $target"

    $sums = Join-Path $tmp 'SHA256SUMS'
    try { Get-Remote "$url/SHA256SUMS" $sums } catch { throw "could not download $url/SHA256SUMS" }
    $expected = Get-ExpectedHash $sums $archive
    if (-not $expected) { throw "$archive is not listed in SHA256SUMS; refusing to install" }
    if ($expected -notmatch '^[0-9a-f]{64}$') { throw "malformed checksum for $archive in SHA256SUMS" }

    $zip = Join-Path $tmp $archive
    try { Get-Remote "$url/$archive" $zip } catch { throw "could not download $url/$archive" }
    $actual = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
      throw "checksum mismatch for $archive (expected $expected, got $actual); nothing installed"
    }

    $extract = Join-Path $tmp 'x'
    Expand-Archive -LiteralPath $zip -DestinationPath $extract
    $bin = Join-Path (Join-Path $extract "branch-tally-$tag-$target") 'branch-tally.exe'
    if (-not (Test-Path -LiteralPath $bin -PathType Leaf)) { throw "$archive does not contain branch-tally.exe" }

    New-Item -ItemType Directory -Force -Path $To | Out-Null
    $To = (Resolve-Path -LiteralPath $To).ProviderPath
    # Stage beside the destination, then rename over it, so an existing binary is replaced in one step.
    $staged = Join-Path $To ".branch-tally.new.$PID.exe"
    Copy-Item -LiteralPath $bin -Destination $staged
    $final = Join-Path $To 'branch-tally.exe'
    try {
      Move-Item -LiteralPath $staged -Destination $final -Force
    } catch {
      throw "cannot replace $final (is branch-tally running?)"
    }
    $staged = $null

    Write-Output "Installed $final"
    $versionOutput = & $final --version
    if ($LASTEXITCODE -ne 0) { throw 'the installed binary did not run' }
    Write-Output $versionOutput

    $onPath = $false
    $sep = [System.IO.Path]::PathSeparator
    foreach ($entry in ($env:PATH -split [regex]::Escape([string]$sep))) {
      $e = $entry.Trim().Trim('"').TrimEnd('\', '/')
      if ($e -and ($e -ieq $To.TrimEnd('\', '/'))) { $onPath = $true; break }
    }
    if (-not $onPath) {
      Write-Output ''
      Write-Output "$To is not on your PATH. To add it for your user (takes effect in new terminals):"
      Write-Output "  [Environment]::SetEnvironmentVariable('Path', `"$To;`" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')"
      Write-Output 'To use it in this terminal only:'
      Write-Output "  `$env:Path = `"$To;`$env:Path`""
    }
  } finally {
    if ($staged -and (Test-Path -LiteralPath $staged)) { Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
}

try {
  Install-BranchTally -Version $Version -To $To
} catch {
  [Console]::Error.WriteLine("install.ps1: error: $($_.Exception.Message)")
  # Run as a file, exit with a status. Under `irm | iex`, exiting would close the user's terminal.
  if ($PSCommandPath) { exit 1 }
  $global:LASTEXITCODE = 1
}
