#!/usr/bin/env pwsh
# Online tests for install.ps1's downloader, Get-Remote, against real GitHub. No arguments; exit 0
# means every case passed, and the number of cases run is printed.
#
# install_test.ps1 is offline: it serves a fake release over file://, so it never runs the https
# path, and in particular never follows a redirect. Every real release download does: github.com
# answers .../releases/download/<tag>/<asset> with a 302 to the asset host. Windows PowerShell 5.1
# and PowerShell 7 report that 302 differently, so CI runs this file under both (`just
# test-install-live powershell` and `just test-install-live pwsh`). It is written for 5.1: no
# PowerShell 7 syntax or APIs.
#
# Get-Remote is a nested function of install.ps1, so it is lifted out of the script's syntax tree
# and defined here, with the same TLS settings and hop limit the installer gives it. Only public
# release assets and the public API are fetched. GH_TOKEN or GITHUB_TOKEN, when set, is sent to the
# API (as the installer does) to avoid the anonymous rate limit, and is never printed.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$installer = [IO.Path]::GetFullPath([IO.Path]::Combine($PSScriptRoot, '..', '..', 'install.ps1'))
$repo = 'IAmSkweetis/branch-tally'
# A release that predates this test and is never deleted: its assets are a stable 302 target.
$tag = 'v0.10.0'
$download = "https://github.com/$repo/releases/download/$tag"

$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($installer, [ref]$null, [ref]$errors)
if ($errors) { throw "install.ps1 does not parse: $($errors[0].Message)" }
$found = @($ast.FindAll({
      param($node)
      ($node -is [System.Management.Automation.Language.FunctionDefinitionAst]) -and ($node.Name -eq 'Get-Remote')
    }, $true))
if ($found.Count -ne 1) { throw "expected one Get-Remote in install.ps1, found $($found.Count)" }
. ([scriptblock]::Create($found[0].Extent.Text))

# As Install-BranchTally sets them before its first download.
try {
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  [Net.ServicePointManager]::SecurityProtocol =
    [Net.ServicePointManager]::SecurityProtocol -bor ([Net.SecurityProtocolType]'Tls13')
} catch {
  $null = $_
}
$script:maxRedirects = 5

$script:cases = 0
$script:failures = 0
$T = Join-Path ([IO.Path]::GetTempPath()) ('bt-install-live-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $T | Out-Null

function Test-Case {
  param([string]$Description, [bool]$Condition, [string]$Detail = '')
  $script:cases++
  if ($Condition) {
    [Console]::Out.WriteLine("ok   $Description")
  } else {
    $script:failures++
    [Console]::Out.WriteLine("FAIL $Description")
    if ($Detail) { [Console]::Out.WriteLine("     $Detail") }
  }
}

function Invoke-Remote {
  # Runs Get-Remote and returns @{ Ok; Error; File }. Never throws.
  param([string]$Url, [hashtable]$Headers = @{}, [int]$TimeoutSec = 0)
  $out = Join-Path $T ([Guid]::NewGuid().ToString('N'))
  try {
    Get-Remote $Url $out $Headers $TimeoutSec
    return @{ Ok = $true; Error = ''; File = $out }
  } catch {
    $message = $_.Exception.Message -replace '(gh[pousr]_|github_pat_)[A-Za-z0-9_]+', '***'
    return @{ Ok = $false; Error = "$($_.Exception.GetType().FullName): $message"; File = $out }
  }
}

function Get-Text {
  param([string]$Path)
  if (Test-Path -LiteralPath $Path) { return [IO.File]::ReadAllText($Path) }
  return ''
}

try {
  [Console]::Out.WriteLine("PowerShell $($PSVersionTable.PSVersion) ($($PSVersionTable.PSEdition))")

  # 1. A release asset: github.com answers with a 302 to another host, which serves the file.
  $res = Invoke-Remote "$download/SHA256SUMS"
  Test-Case 'a release asset is downloaded through its redirect' $res.Ok $res.Error
  $text = Get-Text $res.File
  Test-Case 'the downloaded SHA256SUMS lists the release archives' `
  ($text -match "(?m)^[0-9a-f]{64}  branch-tally-$tag-x86_64-pc-windows-msvc\.zip\r?$") `
    "got $($text.Length) characters"

  # 2. The same through 'latest/download', which redirects twice (to the tag, then to the asset host).
  $res = Invoke-Remote "https://github.com/$repo/releases/latest/download/SHA256SUMS"
  Test-Case 'a latest/download asset is downloaded through both redirects' $res.Ok $res.Error
  Test-Case 'the latest SHA256SUMS lists a Windows archive' `
  ((Get-Text $res.File) -match '(?m)^[0-9a-f]{64}  branch-tally-v[0-9.]+-x86_64-pc-windows-msvc\.zip\r?$')

  # 3. The hop limit applies to real redirects.
  $script:maxRedirects = 0
  $res = Invoke-Remote "$download/SHA256SUMS"
  $script:maxRedirects = 5
  Test-Case 'a redirect past the hop limit fails' (-not $res.Ok)
  Test-Case 'the hop-limit failure says so' ($res.Error -like '*too many redirects*') $res.Error

  # 4. A missing asset is a failure that names the status, and leaves no file behind as a success.
  $res = Invoke-Remote "$download/no-such-asset"
  Test-Case 'a missing asset fails' (-not $res.Ok)
  Test-Case 'the missing-asset failure names HTTP 404' ($res.Error -like '*HTTP status 404*') $res.Error

  # 5. The API lookup the installer makes for "latest": headers are sent, JSON comes back.
  $headers = @{ Accept = 'application/vnd.github+json' }
  $token = $env:GH_TOKEN
  if ([string]::IsNullOrEmpty($token)) { $token = $env:GITHUB_TOKEN }
  if (-not [string]::IsNullOrEmpty($token)) { $headers['Authorization'] = "Bearer $token" }
  $res = Invoke-Remote "https://api.github.com/repos/$repo/releases/latest" $headers 30
  Test-Case 'the latest-release API call succeeds with headers and a timeout' $res.Ok $res.Error
  $latest = $null
  if ($res.Ok) { $latest = (ConvertFrom-Json (Get-Text $res.File)).tag_name }
  Test-Case 'the API answer carries a release tag' (($latest -is [string]) -and ($latest -cmatch '^v[0-9]+\.[0-9]+\.[0-9]+$')) "tag_name: $latest"
} finally {
  Remove-Item -LiteralPath $T -Recurse -Force -ErrorAction SilentlyContinue
}

[Console]::Out.WriteLine("$($script:cases) cases, $($script:failures) failed")
if ($script:failures -gt 0) { exit 1 }
exit 0
