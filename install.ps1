<#
.SYNOPSIS
Installs branch-tally from a GitHub release (Windows).

.DESCRIPTION
Downloads the release archive for this machine's OS architecture (x64 or arm64), verifies it against
the release's SHA256SUMS, and installs branch-tally.exe. A checksum mismatch, or an archive missing
from SHA256SUMS, is a hard failure and nothing is installed. Never edits the user PATH: if the install
directory is not on PATH, the exact command to add it is printed.

The checksum is an integrity check: it catches a corrupt or truncated download, and it does not prove
who built the archive, because the checksum file comes from the same place as the archive. Only the
build-provenance attestation proves origin. After the checksum check this script verifies it, using
the first of these that is available:

  1. gh, logged in:  gh attestation verify <archive> --repo IAmSkweetis/branch-tally `
                       --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml `
                       --source-ref refs/heads/main `
                       --cert-oidc-issuer https://token.actions.githubusercontent.com
  2. cosign:         verifies the release's provenance.sigstore.json with an exact
                     --certificate-identity
                     https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main
                     and --certificate-oidc-issuer https://token.actions.githubusercontent.com
  3. neither:        warns that provenance was NOT verified, prints the command to run, and installs.
                     -RequireAttestation turns this warning into a failure.

A verification that runs and fails is always a hard failure: nothing is installed.

One line:  irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex

`irm | iex` cannot pass parameters, so each has an environment variable. A parameter overrides it.

  -Version             BRANCH_TALLY_INSTALL_VERSION             vX.Y.Z, or latest (the default)
  -To                  BRANCH_TALLY_INSTALL_DIR                 default: %USERPROFILE%\.local\bin
  -RequireAttestation  BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION 1, true or yes: fail unless provenance
                                                                was verified
                       BRANCH_TALLY_INSTALL_BASE_URL            release mirror, https:// or file://
                                                                only (default: the GitHub releases URL
                                                                of the repository). The effective
                                                                source is printed when it is set.
                       BRANCH_TALLY_INSTALL_API_URL             where "latest" is looked up, https://
                                                                or file:// only (default:
                                                                https://api.github.com). Printed when
                                                                set. Trusted as much as the mirror.
                       GH_TOKEN, GITHUB_TOKEN                   when set (GH_TOKEN first), sent as a
                                                                bearer token to the API lookup only.
                                                                Never printed.

With no -Version, the latest tag is read from GitHub's API
(https://api.github.com/repos/IAmSkweetis/branch-tally/releases/latest), even when a mirror is set,
because a mirror serving an older signed release as "latest" would roll you back. If the API cannot
be reached, is rate limited or answers with something unusable, this script falls back to the
mirror's latest/download/SHA256SUMS and warns on stderr that "latest" was not confirmed by GitHub,
so an older release could be served. Pin -Version vX.Y.Z to avoid it. The fallback is allowed with
-RequireAttestation too, because the archive is still verified.

Downloads follow redirects by hand and refuse any hop that is not https://, on Windows PowerShell
5.1 (which would otherwise follow https to http) and on PowerShell 7 alike.

.PARAMETER Version
Release to install, such as v0.4.0. Default: latest.

.PARAMETER To
Directory to install into. Default: %USERPROFILE%\.local\bin.

.PARAMETER RequireAttestation
Fail unless the archive's build provenance was verified with gh or cosign. Without it, a machine with
neither installs with a warning.

#>
param(
  [string]$Version,
  [string]$To,
  [switch]$RequireAttestation
)

function Install-BranchTally {
  param(
    [string]$Version,
    [string]$To,
    [bool]$RequireAttestation
  )

  Set-StrictMode -Version Latest
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'

  $repo = 'IAmSkweetis/branch-tally'
  $defaultBase = "https://github.com/$repo/releases"
  $defaultApi = 'https://api.github.com'
  $maxRedirects = 5
  $issuer = 'https://token.actions.githubusercontent.com'
  # The signer must be this repository's release workflow, run from main. release.yml starts from
  # workflow_run on main, and a manual dispatch on another branch is not a release.
  $signerWorkflow = "$repo/.github/workflows/release.yml"
  $sourceRef = 'refs/heads/main'
  $signerIdentity = "https://github.com/$signerWorkflow@$sourceRef"
  # The gh flags that hold an attestation to that signer, for the command we run and the one we print.
  $ghPins = @('--repo', $repo, '--signer-workflow', $signerWorkflow, '--source-ref', $sourceRef, '--cert-oidc-issuer', $issuer)

  if ([string]::IsNullOrEmpty($Version)) { $Version = $env:BRANCH_TALLY_INSTALL_VERSION }
  if ([string]::IsNullOrEmpty($Version)) { $Version = 'latest' }
  if ([string]::IsNullOrEmpty($To)) { $To = $env:BRANCH_TALLY_INSTALL_DIR }
  $base = $env:BRANCH_TALLY_INSTALL_BASE_URL
  if ([string]::IsNullOrEmpty($base)) { $base = $defaultBase }
  $base = $base.TrimEnd('/')
  $api = $env:BRANCH_TALLY_INSTALL_API_URL
  if ([string]::IsNullOrEmpty($api)) { $api = $defaultApi }
  $api = $api.TrimEnd('/')
  if (-not $RequireAttestation) {
    $RequireAttestation = $env:BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION -match '^(1|true|yes)$'
  }

  # The release source must be https:// or file://, as for `branch-tally update`. Plain http is
  # refused: it would let anyone on the path replace both the archive and its checksum.
  if ($base -notmatch '^(https|file)://') {
    $shown = $base -replace '[^A-Za-z0-9._/:@=+-]', ''
    throw "BRANCH_TALLY_INSTALL_BASE_URL must start with https:// or file:// (got '$shown')"
  }
  if ($base -ne $defaultBase) { Write-Output "Using release mirror: $base" }
  # The latest-release lookup has the same scheme rule, and carries the same trust as the mirror.
  if ($api -notmatch '^(https|file)://') {
    $shown = $api -replace '[^A-Za-z0-9._/:@=+-]', ''
    throw "BRANCH_TALLY_INSTALL_API_URL must start with https:// or file:// (got '$shown')"
  }
  if ($api -ne $defaultApi) { Write-Output "Using release API: $api" }

  if ($Version -ne 'latest') {
    if (-not $Version.StartsWith('v')) { $Version = "v$Version" }
    # \z, not $: in .NET a $ also matches before a trailing newline.
    if ($Version -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+\z') {
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
    # TLS 1.2 and newer only: no SSL3, TLS 1.0 or 1.1 fallback.
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    [Net.ServicePointManager]::SecurityProtocol =
      [Net.ServicePointManager]::SecurityProtocol -bor ([Net.SecurityProtocolType]'Tls13')
  } catch {
    # Newer runtimes negotiate TLS themselves, and Windows PowerShell 5.1 may lack the Tls13 value.
    $null = $_
  }

  function Get-Remote {
    # Downloads $Url to $OutFile. https:// is followed by hand: Windows PowerShell 5.1 follows an
    # https to http redirect on its own, so automatic redirects are off (MaximumRedirection 0) and
    # every hop is checked to be https:// before it is requested, up to $maxRedirects hops, on 5.1
    # and 7 alike. file:// is a plain copy (the test and local-mirror seam). The scheme test is
    # case-insensitive and ordinal, the same as the mirror check above. Headers (a token) are
    # dropped when a redirect leaves the original host.
    param([string]$Url, [string]$OutFile, [hashtable]$Headers = @{}, [int]$TimeoutSec = 0)
    if ($Url.StartsWith('file://', [System.StringComparison]::OrdinalIgnoreCase)) {
      # Invoke-WebRequest has no file:// support.
      Copy-Item -LiteralPath ([Uri]$Url).LocalPath -Destination $OutFile
      return
    }
    $current = $Url
    $firstHost = ([Uri]$Url).Host
    for ($hop = 0; $hop -le $maxRedirects; $hop++) {
      if (-not $current.StartsWith('https://', [System.StringComparison]::OrdinalIgnoreCase)) {
        $shown = $current -replace '[^A-Za-z0-9._/:@=+-]', ''
        throw "refusing to follow a non-https redirect to '$shown'"
      }
      $sendHeaders = @{}
      if (([Uri]$current).Host -eq $firstHost) { $sendHeaders = $Headers }
      $status = 0
      $location = $null
      try {
        # Windows PowerShell 5.1 has no timeout by default. $TimeoutSec applies to each hop, and a
        # timeout throws without a response, so it reaches the caller as a failure.
        $request = @{ Uri = $current; OutFile = $OutFile; UseBasicParsing = $true; MaximumRedirection = 0; Headers = $sendHeaders }
        if ($TimeoutSec -gt 0) { $request['TimeoutSec'] = $TimeoutSec }
        Invoke-WebRequest @request
        return
      } catch {
        # 5.1 throws a WebException (HttpWebResponse, WebHeaderCollection). 7 throws an
        # HttpResponseException (HttpResponseMessage, with a typed Location).
        $response = $null
        if ($_.Exception.PSObject.Properties['Response']) { $response = $_.Exception.Response }
        if ($null -eq $response) { throw }
        $status = [int]$response.StatusCode
        if ($response.Headers -is [System.Net.WebHeaderCollection]) {
          $location = $response.Headers['Location']
        } elseif ($null -ne $response.Headers.Location) {
          $location = $response.Headers.Location.ToString()
        }
      }
      if ((@(301, 302, 303, 307, 308) -notcontains $status) -or [string]::IsNullOrEmpty($location)) {
        throw "download failed with HTTP status $status"
      }
      $current = ([Uri]::new([Uri]$current, $location)).AbsoluteUri
    }
    throw "too many redirects (more than $maxRedirects) fetching $Url"
  }

  function Get-LatestTagFromApi {
    # The latest release tag as GitHub reports it, or $null when the API cannot be reached, is rate
    # limited, or answers with anything that is not a vMAJOR.MINOR.PATCH tag_name. Never throws and
    # never prints the token.
    param([string]$Dir)
    try {
      $headers = @{ Accept = 'application/vnd.github+json' }
      $token = $env:GH_TOKEN
      if ([string]::IsNullOrEmpty($token)) { $token = $env:GITHUB_TOKEN }
      if (-not [string]::IsNullOrEmpty($token)) {
        if ($token -match '^[A-Za-z0-9_.~+/=-]+\z') {
          $headers['Authorization'] = "Bearer $token"
        } else {
          [Console]::Error.WriteLine('install.ps1: warning: ignoring GH_TOKEN/GITHUB_TOKEN: it contains characters a token never has')
        }
      }
      $json = Join-Path $Dir 'latest.json'
      # 30 seconds per hop, like install.sh. A timeout lands in the catch below and so in the fallback.
      Get-Remote "$api/repos/$repo/releases/latest" $json $headers 30
      $doc = [System.IO.File]::ReadAllText($json) | ConvertFrom-Json
      $found = $doc.tag_name
      if (($found -is [string]) -and ($found -cmatch '^v[0-9]+\.[0-9]+\.[0-9]+\z')) { return $found }
    } catch {
      $null = $_
    }
    return $null
  }

  function Invoke-Native {
    # Runs a native command, capturing its exit code and combined output without letting stderr
    # text trip $ErrorActionPreference = 'Stop' (Windows PowerShell 5.1 does that).
    param([string]$File, [string[]]$ArgumentList)
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
      $text = (& $File @ArgumentList 2>&1 | Out-String)
      return @{ Code = $LASTEXITCODE; Out = $text }
    } finally {
      $ErrorActionPreference = $old
    }
  }

  function Write-Unverified {
    param([string]$Reason)
    if ($RequireAttestation) {
      throw "build provenance could not be verified ($Reason) and -RequireAttestation is set; nothing installed. Install gh or cosign, or run: gh attestation verify $archive $($ghPins -join ' ')"
    }
    [Console]::Error.WriteLine("install.ps1: warning: provenance NOT verified: $Reason.")
    [Console]::Error.WriteLine('The checksum is an integrity check only; it does not prove who built this archive.')
    [Console]::Error.WriteLine("To verify it yourself, download $url/$archive and run:")
    [Console]::Error.WriteLine("  gh attestation verify $archive $($ghPins -join ' ')")
  }

  function Confirm-Provenance {
    # Verifies the downloaded archive's attestation. Returns when verified, or when no tool could
    # check it and it is not required. Throws when a verification ran and failed.
    param([string]$Zip, [string]$Dir)
    $gh = Get-Command gh -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($gh) {
      $auth = Invoke-Native $gh.Source @('auth', 'status')
      if ($auth.Code -eq 0) {
        $r = Invoke-Native $gh.Source (@('attestation', 'verify', $Zip) + $ghPins)
        if ($r.Code -eq 0) {
          Write-Output "Provenance verified with gh: $archive was built by $repo"
          return
        }
        [Console]::Error.Write($r.Out)
        throw "attestation verification failed for $archive; nothing installed"
      }
    }
    $cosign = Get-Command cosign -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cosign) {
      $bundle = Join-Path $Dir 'provenance.sigstore.json'
      $haveBundle = $true
      try { Get-Remote "$url/provenance.sigstore.json" $bundle } catch { $haveBundle = $false }
      if (-not $haveBundle) {
        Write-Unverified 'cosign is installed but the release has no provenance.sigstore.json'
        return
      }
      $r = Invoke-Native $cosign.Source @(
        'verify-blob-attestation', '--bundle', $bundle, '--new-bundle-format',
        '--type', 'slsaprovenance1',
        '--certificate-identity', $signerIdentity,
        '--certificate-oidc-issuer', $issuer,
        $Zip)
      if ($r.Code -eq 0) {
        Write-Output "Provenance verified with cosign: $archive was built by $signerWorkflow on $sourceRef"
        return
      }
      [Console]::Error.Write($r.Out)
      throw "attestation verification failed for $archive; nothing installed"
    }
    Write-Unverified 'neither gh (logged in) nor cosign is available'
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
    # Resolve "latest" to a tag. GitHub's API is the authority, even with a mirror: a mirror that
    # serves an older signed release as "latest" would roll users back, and a signature proves a
    # release is genuine, not that it is the newest. Everything after comes from download/<tag>/,
    # never latest/, so a release landing mid-install cannot mix versions.
    if ($Version -eq 'latest') {
      $tag = Get-LatestTagFromApi $tmp
      if (-not $tag) {
        [Console]::Error.WriteLine("install.ps1: warning: could not read the latest release from $api/repos/$repo/releases/latest (unreachable,")
        [Console]::Error.WriteLine("rate limited or an unusable answer). Falling back to the mirror's latest/download/SHA256SUMS.")
        [Console]::Error.WriteLine("'Latest' is from the mirror and was not confirmed by GitHub, so an older release could be")
        [Console]::Error.WriteLine('served. To choose the release yourself, run: install.ps1 -Version vX.Y.Z')
        $latestSums = Join-Path $tmp 'latest.sums'
        try {
          Get-Remote "$base/latest/download/SHA256SUMS" $latestSums
        } catch {
          throw "could not download $base/latest/download/SHA256SUMS"
        }
        $pattern = '^branch-tally-(v[0-9]+\.[0-9]+\.[0-9]+)-' + [regex]::Escape($target) + '\.zip\z'
        foreach ($line in [System.IO.File]::ReadAllLines($latestSums)) {
          $parts = $line.Trim() -split '\s+'
          if ($parts.Count -ge 2 -and $parts[1].TrimStart('*') -match $pattern) {
            $tag = $Matches[1]
            break
          }
        }
        if (-not $tag) { throw "the latest release has no build for $target" }
      }
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
    if ($expected -notmatch '^[0-9a-f]{64}\z') { throw "malformed checksum for $archive in SHA256SUMS" }

    $zip = Join-Path $tmp $archive
    try { Get-Remote "$url/$archive" $zip } catch { throw "could not download $url/$archive" }
    $actual = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
      throw "checksum mismatch for $archive (expected $expected, got $actual); nothing installed"
    }
    Write-Output 'Checksum OK (an integrity check; it does not prove who built the archive)'

    Confirm-Provenance $zip $tmp

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
  Install-BranchTally -Version $Version -To $To -RequireAttestation ([bool]$RequireAttestation)
} catch {
  [Console]::Error.WriteLine("install.ps1: error: $($_.Exception.Message)")
  # Run as a file, exit with a status. Under `irm | iex`, exiting would close the user's terminal.
  if ($PSCommandPath) { exit 1 }
  $global:LASTEXITCODE = 1
}
