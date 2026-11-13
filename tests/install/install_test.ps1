#!/usr/bin/env pwsh
# Tests for install.ps1. A plain PowerShell 7 script with no Pester dependency, no arguments.
# Exit 0 means every case passed. Prints the number of cases run.
#
# install.ps1 only installs on Windows, so the full suite runs there. Elsewhere only the
# unsupported-OS case runs, and the output says the rest were skipped.
#
# Builds a fake release tree under a temp dir and points BRANCH_TALLY_INSTALL_BASE_URL at it over
# file://. The child process gets USERPROFILE, HOME, TEMP, TMP and TMPDIR inside the sandbox, so a
# run never touches the real %USERPROFILE%\.local\bin or the network. The stub binary is compiled
# from C# with the .NET Framework csc.exe that ships with Windows.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$installer = (Resolve-Path (Join-Path $PSScriptRoot '..' '..' 'install.ps1')).ProviderPath
$pwshExe = (Get-Process -Id $PID).Path
$T = Join-Path ([IO.Path]::GetTempPath()) ('bt-install-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $T | Out-Null

$script:cases = 0
$script:failures = 0

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

function Write-NoBom {
  param([string]$Path, [string]$Text)
  [IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding($false)))
}

function Get-SafePath {
  param([string]$Prefix = '')
  $p = "$($script:S.Stubs);$env:windir\System32;$env:windir"
  if ($Prefix) { $p = "$Prefix;$p" }
  return $p
}

function New-Stub {
  # A .cmd stub for gh or cosign in the sandbox that logs its arguments to <name>.log. For gh,
  # $Codes is @(auth status rc, attestation verify rc). For cosign, @(rc).
  param([string]$Name, [int[]]$Codes)
  $body = "@echo off`r`necho %*>>`"%~dp0$Name.log`"`r`n"
  if ($Name -eq 'gh') {
    $body += "if `"%1 %2`"==`"auth status`" exit /b $($Codes[0])`r`n"
    $body += "if `"%1 %2`"==`"attestation verify`" exit /b $($Codes[1])`r`n"
    $body += "exit /b 99`r`n"
  } else {
    $body += "exit /b $($Codes[0])`r`n"
  }
  Write-NoBom (Join-Path $script:S.Stubs "$Name.cmd") $body
}

function Get-StubLog {
  param([string]$Name)
  $f = Join-Path $script:S.Stubs "$Name.log"
  if (Test-Path -LiteralPath $f) { return [IO.File]::ReadAllText($f) }
  return ''
}

function Invoke-Installer {
  # Runs install.ps1 in a child pwsh. Returns @{ Rc; Out; Err }.
  param([string[]]$Arguments = @(), [hashtable]$Env = @{}, [string]$ScriptPath = $installer)
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $pwshExe
  foreach ($a in @('-NoProfile', '-NonInteractive', '-File', $ScriptPath) + $Arguments) {
    $psi.ArgumentList.Add($a)
  }
  $psi.RedirectStandardOutput = $true
  $psi.RedirectStandardError = $true
  $psi.UseShellExecute = $false
  foreach ($k in @($psi.Environment.Keys | Where-Object { $_ -like 'BRANCH_TALLY_INSTALL_*' })) {
    $psi.Environment.Remove($k) | Out-Null
  }
  $psi.Environment['USERPROFILE'] = $script:S.Home
  $psi.Environment['HOME'] = $script:S.Home
  $psi.Environment['TEMP'] = $script:S.Tmp
  $psi.Environment['TMP'] = $script:S.Tmp
  $psi.Environment['TMPDIR'] = $script:S.Tmp
  $psi.Environment['BRANCH_TALLY_INSTALL_BASE_URL'] = ([Uri]$script:S.Release).AbsoluteUri
  # A PATH of the sandbox's stubs plus System32 (for cmd.exe), so the host's own gh and cosign can
  # never leak into a case.
  $psi.Environment['PATH'] = Get-SafePath
  foreach ($k in $Env.Keys) { $psi.Environment[$k] = [string]$Env[$k] }
  $p = [System.Diagnostics.Process]::Start($psi)
  $outTask = $p.StandardOutput.ReadToEndAsync()
  $errTask = $p.StandardError.ReadToEndAsync()
  $p.WaitForExit()
  return @{ Rc = $p.ExitCode; Out = $outTask.Result; Err = $errTask.Result }
}

function Initialize-Sandbox {
  param([string]$Name)
  $root = Join-Path $T $Name
  $sandbox = @{
    Root    = $root
    Home    = Join-Path $root 'home'
    Tmp     = Join-Path $root 'tmp'
    Release = Join-Path $root 'release'
    Dest    = Join-Path $root 'dest'
    Stubs   = Join-Path $root 'stubs'
  }
  foreach ($dir in @($sandbox.Home, $sandbox.Tmp, $sandbox.Release, $sandbox.Stubs)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $script:S = $sandbox
}

function Get-RelativeFile {
  # Paths (never contents) of every file under Dir, relative to it, sorted.
  param([string]$Dir)
  $full = (Resolve-Path -LiteralPath $Dir).ProviderPath
  $paths = @(Get-ChildItem -LiteralPath $full -Recurse -Force -File -ErrorAction SilentlyContinue |
      ForEach-Object { $_.FullName.Substring($full.Length).TrimStart('\', '/') })
  return @($paths | Sort-Object)
}

function Test-Empty {
  param([string]$Dir)
  return -not (Get-ChildItem -LiteralPath $Dir -Force | Select-Object -First 1)
}

# ---------------------------------------------------------------------------------------------
if (-not $IsWindows) {
  Initialize-Sandbox 'nonwin'
  $res = Invoke-Installer @('-To', $script:S.Dest)
  Test-Case 'non-Windows: exits non-zero' ($res.Rc -ne 0)
  Test-Case 'non-Windows: points at install.sh' ($res.Err -like '*install.sh*')
  Test-Case 'non-Windows: installs nothing' (-not (Test-Path (Join-Path $script:S.Dest 'branch-tally.exe')))
  [Console]::Out.WriteLine('')
  [Console]::Out.WriteLine("$($script:cases) cases run, $($script:failures) failed (SKIPPED: the rest need Windows)")
  Remove-Item -LiteralPath $T -Recurse -Force -ErrorAction SilentlyContinue
  if ($script:failures -eq 0 -and $script:cases -gt 0) { exit 0 } else { exit 1 }
}

# ---- Windows from here ----
$osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$target = if ($osArch -eq 'Arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }

$csc = $null
foreach ($f in @('Framework64', 'Framework')) {
  $c = Join-Path $env:windir "Microsoft.NET\$f\v4.0.30319\csc.exe"
  if (Test-Path $c) { $csc = $c; break }
}
if (-not $csc) {
  [Console]::Out.WriteLine('FAIL csc.exe not found: cannot build the stub binary')
  Remove-Item -LiteralPath $T -Recurse -Force -ErrorAction SilentlyContinue
  exit 1
}

$stubCache = @{}
function Get-StubExe {
  param([string]$Ver)
  if (-not $stubCache.ContainsKey($Ver)) {
    $src = Join-Path $T "stub-$Ver.cs"
    $exe = Join-Path $T "stub-$Ver.exe"
    Write-NoBom $src "class P { static void Main() { System.Console.WriteLine(`"branch-tally $Ver`"); } }"
    & $csc /nologo /target:exe "/out:$exe" $src | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "csc failed for $Ver" }
    $stubCache[$Ver] = $exe
  }
  return $stubCache[$Ver]
}

function Build-Release {
  # One release under <root>/download/<tag>, laid out like scripts/package.sh's zip.
  param([string]$Root, [string]$Tag, [string]$Tgt, [string]$Ver)
  $name = "branch-tally-$Tag-$Tgt"
  $stage = Join-Path $T 'stage'
  if (Test-Path $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
  New-Item -ItemType Directory -Path (Join-Path $stage $name) | Out-Null
  $relDir = Join-Path (Join-Path $Root 'download') $Tag
  New-Item -ItemType Directory -Path $relDir -Force | Out-Null
  Copy-Item (Get-StubExe $Ver) (Join-Path (Join-Path $stage $name) 'branch-tally.exe')
  Write-NoBom (Join-Path (Join-Path $stage $name) 'LICENSE') 'LICENSE'
  Write-NoBom (Join-Path (Join-Path $stage $name) 'README.md') 'README'
  $zip = Join-Path $relDir "$name.zip"
  Compress-Archive -Path (Join-Path $stage $name) -DestinationPath $zip
  $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
  $sums = Join-Path $relDir 'SHA256SUMS'
  $existing = if (Test-Path $sums) { [IO.File]::ReadAllText($sums) } else { '' }
  Write-NoBom $sums ($existing + "$hash  $name.zip`n")
  Write-NoBom (Join-Path $relDir 'provenance.sigstore.json') '{"fake":"bundle"}'
  Remove-Item -LiteralPath $stage -Recurse -Force
}

function Build-Latest {
  param([string]$Root, [string]$Tag)
  $latest = Join-Path $Root 'latest'
  if (Test-Path $latest) { Remove-Item -LiteralPath $latest -Recurse -Force }
  New-Item -ItemType Directory -Path $latest | Out-Null
  Copy-Item -LiteralPath (Join-Path (Join-Path $Root 'download') $Tag) -Destination (Join-Path $latest 'download') -Recurse
}

function Build-BadChecksum {
  param([string]$Root, [string]$Tag)
  $sums = Join-Path (Join-Path (Join-Path $Root 'download') $Tag) 'SHA256SUMS'
  $text = [IO.File]::ReadAllText($sums) -replace '(?m)^[0-9a-f]{64}', ('0' * 64)
  Write-NoBom $sums $text
}

function Get-StubOutput {
  param([string]$Exe)
  return ((& $Exe) -join '')
}

# 1. Happy path: latest, custom dir.
Initialize-Sandbox 'happy'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 'happy path exits 0' ($res.Rc -eq 0)
Test-Case 'happy path installs branch-tally.exe' (Test-Path (Join-Path $D 'branch-tally.exe'))
Test-Case 'happy path prints the installed path' ($res.Out -like "*$(Join-Path $D 'branch-tally.exe')*")
Test-Case 'happy path prints the version' ($res.Out -like '*branch-tally 2.0.0*')
Test-Case 'happy path leaves no temp files in TEMP' (Test-Empty $script:S.Tmp)
Test-Case 'happy path leaves only the binary in the dir' ((@(Get-ChildItem -LiteralPath $D -Force).Name) -join ',' -eq 'branch-tally.exe')

# 2. Pinned version through the environment, while latest is newer.
Initialize-Sandbox 'pinned'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @() @{ BRANCH_TALLY_INSTALL_DIR = $D; BRANCH_TALLY_INSTALL_VERSION = 'v1.0.0' }
Test-Case 'env-pinned version exits 0' ($res.Rc -eq 0)
Test-Case 'env-pinned version installs v1.0.0' ($res.Out -like '*branch-tally 1.0.0*')

# 2b. Parameters override env.
$ignored = Join-Path $script:S.Root 'ignored'
$res = Invoke-Installer @('-Version', 'v1.0.0', '-To', $D) @{ BRANCH_TALLY_INSTALL_DIR = $ignored; BRANCH_TALLY_INSTALL_VERSION = 'v2.0.0' }
Test-Case '-Version overrides env' ($res.Out -like '*branch-tally 1.0.0*')
Test-Case '-To overrides env' (-not (Test-Path $ignored))

# 2c. Version without the leading v.
$res = Invoke-Installer @('-Version', '1.0.0', '-To', $D)
Test-Case 'version without v is accepted' ($res.Rc -eq 0)

# 2d. Unknown version.
$res = Invoke-Installer @('-Version', 'v9.9.9', '-To', $D)
Test-Case 'unknown version exits non-zero' ($res.Rc -ne 0)

# 2e. Malformed version.
$res = Invoke-Installer @('-Version', 'v1;rm', '-To', $D)
Test-Case 'malformed version exits non-zero' ($res.Rc -ne 0)
Test-Case 'malformed version says so' ($res.Err -like '*version*')

# 3. Checksum mismatch: hard fail, nothing installed.
Initialize-Sandbox 'badsum'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-BadChecksum $R 'v2.0.0'
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 'checksum mismatch exits non-zero' ($res.Rc -ne 0)
Test-Case 'checksum mismatch installs nothing' (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
Test-Case 'checksum mismatch says so on stderr' ($res.Err -like '*checksum*')
Test-Case 'checksum mismatch leaves no temp files' (Test-Empty $script:S.Tmp)

# 3b. A failed upgrade keeps the old binary.
Initialize-Sandbox 'keepold'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Latest $R 'v1.0.0'
$null = Invoke-Installer @('-To', $D)
Build-Release $R 'v3.0.0' $target '3.0.0'
Build-BadChecksum $R 'v3.0.0'
Build-Latest $R 'v3.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 'failed upgrade exits non-zero' ($res.Rc -ne 0)
Test-Case 'failed upgrade keeps the old binary' ((Get-StubOutput (Join-Path $D 'branch-tally.exe')) -eq 'branch-tally 1.0.0')

# 4. Target missing from SHA256SUMS.
Initialize-Sandbox 'notarget'
$R = $script:S.Release; $D = $script:S.Dest
$otherTarget = if ($target -like 'x86_64*') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
Build-Release $R 'v2.0.0' $otherTarget '2.0.0'
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 'target missing from SHA256SUMS exits non-zero' ($res.Rc -ne 0)
Test-Case 'target missing installs nothing' (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
$res = Invoke-Installer @('-Version', 'v2.0.0', '-To', $D)
Test-Case 'pinned target missing exits non-zero' ($res.Rc -ne 0)

# 4b. Archive present but no SHA256SUMS line for it.
Initialize-Sandbox 'nosumline'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Write-NoBom (Join-Path (Join-Path (Join-Path $R 'download') 'v2.0.0') 'SHA256SUMS') ''
$res = Invoke-Installer @('-Version', 'v2.0.0', '-To', $D)
Test-Case 'missing SHA256SUMS line exits non-zero' ($res.Rc -ne 0)
Test-Case 'missing SHA256SUMS line installs nothing' (-not (Test-Path (Join-Path $D 'branch-tally.exe')))

# 4c. No SHA256SUMS file.
Remove-Item (Join-Path (Join-Path (Join-Path $R 'download') 'v2.0.0') 'SHA256SUMS')
$res = Invoke-Installer @('-Version', 'v2.0.0', '-To', $D)
Test-Case 'missing SHA256SUMS file exits non-zero' ($res.Rc -ne 0)

# 5. PATH hint.
Initialize-Sandbox 'pathhint'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
# A control child (a no-op script, same environment) shows what the runtime alone leaves in HOME, so
# the profile check below measures the installer and not pwsh/.NET.
$noop = Join-Path $T 'noop.ps1'
Write-NoBom $noop 'exit 0'
$null = Invoke-Installer @() @{} $noop
$homeBefore = Get-RelativeFile $script:S.Home
$res = Invoke-Installer @('-To', $D)
Test-Case 'PATH hint shown when dir is not on PATH' ($res.Out -like '*is not on your PATH*' -and $res.Out -like "*$D*")
$res = Invoke-Installer @('-To', $D) @{ PATH = (Get-SafePath $D) }
Test-Case 'no PATH hint when dir is on PATH' ($res.Out -notlike '*is not on your PATH*')
$res = Invoke-Installer @('-To', $D) @{ PATH = (Get-SafePath "$($D.ToUpperInvariant())\") }
Test-Case 'no PATH hint for a case-different, trailing-backslash entry' ($res.Out -notlike '*is not on your PATH*')
$homeAfter = Get-RelativeFile $script:S.Home
$homeAdded = @($homeAfter | Where-Object { $homeBefore -notcontains $_ })
Test-Case 'installer adds nothing to the user profile directory' ($homeAdded.Count -eq 0) ('unexpected paths: ' + ($homeAdded -join '; '))
$profileFiles = @($homeAfter | Where-Object { $_ -like '*profile*.ps1' })
Test-Case 'no PowerShell profile file was created' ($profileFiles.Count -eq 0) ('profile paths: ' + ($profileFiles -join '; '))

# 6. Re-install overwrites.
Initialize-Sandbox 'reinstall'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v1.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 'first install is v1' ($res.Out -like '*branch-tally 1.0.0*')
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 're-install exits 0' ($res.Rc -eq 0)
Test-Case 're-install overwrites with v2' ((Get-StubOutput (Join-Path $D 'branch-tally.exe')) -eq 'branch-tally 2.0.0')

# 7. Default directory is %USERPROFILE%\.local\bin.
Initialize-Sandbox 'defaultdir'
$R = $script:S.Release
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @()
Test-Case 'default dir is USERPROFILE\.local\bin' (Test-Path (Join-Path $script:S.Home '.local\bin\branch-tally.exe'))

# 8. There is no attestation parameter: the installer needs nothing but built-in PowerShell.
Initialize-Sandbox 'noattest'
$res = Invoke-Installer @('-To', $script:S.Dest, '-VerifyAttestation')
Test-Case '-VerifyAttestation is not a parameter' ($res.Rc -ne 0)

# 9. Provenance. The checksum is an integrity check; only the attestation proves origin.
# 9a. gh logged in and the attestation verifies.
Initialize-Sandbox 'gh_pass'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
New-Stub 'gh' @(0, 0)
$res = Invoke-Installer @('-To', $D)
$ghLog = Get-StubLog 'gh'
Test-Case 'gh pass exits 0' ($res.Rc -eq 0)
Test-Case 'gh pass installs' (Test-Path (Join-Path $D 'branch-tally.exe'))
Test-Case 'gh pass ran attestation verify' ($ghLog -like '*attestation verify*')
Test-Case 'gh pass pins the repository' ($ghLog -like '*--repo IAmSkweetis/branch-tally*')
Test-Case 'gh pass verifies the archive' ($ghLog -like "*branch-tally-v2.0.0-$target.zip*")
Test-Case 'gh pass says provenance was verified' ($res.Out -like '*Provenance verified*')

# 9b. gh logged in and verification fails: hard failure, no cosign fallback.
Initialize-Sandbox 'gh_fail'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
New-Stub 'gh' @(0, 1)
New-Stub 'cosign' @(0)
$res = Invoke-Installer @('-To', $D)
Test-Case 'gh fail exits non-zero' ($res.Rc -ne 0)
Test-Case 'gh fail installs nothing' (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
Test-Case 'gh fail says attestation' ($res.Err -like '*attestation*')
Test-Case 'gh fail does not fall back to cosign' ((Get-StubLog 'cosign') -eq '')
Test-Case 'gh fail leaves no temp files' (Test-Empty $script:S.Tmp)

# 9c. A checksum failure stops before any provenance tool runs.
Initialize-Sandbox 'gh_after_sum'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-BadChecksum $R 'v2.0.0'
Build-Latest $R 'v2.0.0'
New-Stub 'gh' @(0, 0)
$res = Invoke-Installer @('-To', $D)
Test-Case 'bad checksum exits non-zero' ($res.Rc -ne 0)
Test-Case 'bad checksum never reaches gh attestation' ((Get-StubLog 'gh') -notlike '*attestation*')

# 9d. gh not logged in, cosign verifies: identity and issuer are pinned.
Initialize-Sandbox 'cosign_pass'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
New-Stub 'gh' @(1, 0)
New-Stub 'cosign' @(0)
$res = Invoke-Installer @('-To', $D)
$cosignLog = Get-StubLog 'cosign'
Test-Case 'cosign pass exits 0' ($res.Rc -eq 0)
Test-Case 'cosign pass installs' (Test-Path (Join-Path $D 'branch-tally.exe'))
Test-Case 'cosign pass did not run gh attestation verify' ((Get-StubLog 'gh') -notlike '*attestation*')
Test-Case 'cosign pass uses the bundle' ($cosignLog -like '*provenance.sigstore.json*')
Test-Case 'cosign pass pins the workflow identity' ($cosignLog -like '*/IAmSkweetis/branch-tally/\.github/workflows/release\.yml@*')
Test-Case 'cosign pass pins the issuer' ($cosignLog -like '*https://token.actions.githubusercontent.com*')
Test-Case 'cosign pass says provenance was verified' ($res.Out -like '*Provenance verified*')

# 9e. cosign verification fails: hard failure.
Initialize-Sandbox 'cosign_fail'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
New-Stub 'cosign' @(1)
$res = Invoke-Installer @('-To', $D)
Test-Case 'cosign fail exits non-zero' ($res.Rc -ne 0)
Test-Case 'cosign fail installs nothing' (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
Test-Case 'cosign fail says attestation' ($res.Err -like '*attestation*')

# 9f. cosign present but the release has no bundle.
Initialize-Sandbox 'cosign_nobundle'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Remove-Item (Join-Path (Join-Path (Join-Path $R 'download') 'v2.0.0') 'provenance.sigstore.json')
Build-Latest $R 'v2.0.0'
New-Stub 'cosign' @(0)
$res = Invoke-Installer @('-To', $D)
Test-Case 'no bundle installs with a warning' ($res.Rc -eq 0 -and $res.Err -like '*NOT verified*')
Test-Case 'no bundle never ran cosign' ((Get-StubLog 'cosign') -eq '')
Remove-Item -LiteralPath $D -Recurse -Force
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case 'no bundle with -RequireAttestation fails' ($res.Rc -ne 0)

# 9g. Neither tool: warn, print the command, install. Strict mode fails instead.
Initialize-Sandbox 'unavailable'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 'unavailable exits 0' ($res.Rc -eq 0)
Test-Case 'unavailable installs' (Test-Path (Join-Path $D 'branch-tally.exe'))
Test-Case 'unavailable warns provenance was not verified' ($res.Err -like '*NOT verified*')
Test-Case 'unavailable says the checksum is only an integrity check' ($res.Err -like '*integrity*')
Test-Case 'unavailable prints the gh command' ($res.Err -like '*gh attestation verify*--repo IAmSkweetis/branch-tally*')
Remove-Item -LiteralPath $D -Recurse -Force
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case '-RequireAttestation with no tool exits non-zero' ($res.Rc -ne 0)
Test-Case '-RequireAttestation with no tool installs nothing' (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
Test-Case '-RequireAttestation says why' ($res.Err -like '*attestation*')
$res = Invoke-Installer @('-To', $D) @{ BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION = '1' }
Test-Case 'BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=1 exits non-zero' ($res.Rc -ne 0)
Test-Case 'BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=1 installs nothing' (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
$res = Invoke-Installer @('-To', $D) @{ BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION = '0' }
Test-Case 'BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=0 does not require it' ($res.Rc -eq 0)

# 9h. Strict mode with a tool that verifies succeeds.
Initialize-Sandbox 'strict_ok'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
New-Stub 'gh' @(0, 0)
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case '-RequireAttestation with a passing gh exits 0' ($res.Rc -eq 0)

# 9i. gh on PATH but not logged in, no cosign: unavailable.
Initialize-Sandbox 'gh_loggedout'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
New-Stub 'gh' @(1, 0)
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case 'logged-out gh with no cosign is unavailable (strict fails)' ($res.Rc -ne 0)
Test-Case 'logged-out gh never ran attestation verify' ((Get-StubLog 'gh') -notlike '*attestation*')

# 10. F-06: mirror URL scheme, and the effective source.
Initialize-Sandbox 'scheme'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
foreach ($bad in @('http://mirror.example.test', 'ftp://mirror.example.test', 'mirror.example.test')) {
  $res = Invoke-Installer @('-To', $D) @{ BRANCH_TALLY_INSTALL_BASE_URL = $bad }
  Test-Case "mirror '$bad' is rejected" ($res.Rc -ne 0)
  Test-Case "mirror '$bad' says https or file" ($res.Err -like '*https://*')
  Test-Case "mirror '$bad' installs nothing" (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
}
$res = Invoke-Installer @('-To', $D)
Test-Case 'a file:// mirror prints the effective source' ($res.Out -like '*Using release mirror: file://*')

# 11. F-06: a tag read from the remote SHA256SUMS must be vMAJOR.MINOR.PATCH.
Initialize-Sandbox 'badtag'
$R = $script:S.Release; $D = $script:S.Dest
foreach ($badname in @("branch-tally-vevil-$target.zip", "branch-tally-v1.2-$target.zip", "branch-tally-v1.2.3.4-$target.zip")) {
  if (Test-Path $R) { Remove-Item -LiteralPath $R -Recurse -Force }
  $dir = Join-Path (Join-Path $R 'latest') 'download'
  New-Item -ItemType Directory -Path $dir -Force | Out-Null
  Write-NoBom (Join-Path $dir 'SHA256SUMS') (('0' * 64) + "  $badname`n")
  $res = Invoke-Installer @('-To', $D)
  Test-Case "remote tag in '$badname' is rejected" ($res.Rc -ne 0)
  Test-Case "remote tag in '$badname' installs nothing" (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
}

# 12. The help names the new parameter.
$helpText = [IO.File]::ReadAllText($installer)
Test-Case 'help documents -RequireAttestation' ($helpText -like '*-RequireAttestation*' -and $helpText -like '*BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION*')
Test-Case 'help calls the checksum an integrity check' ($helpText -like '*integrity check*')

[Console]::Out.WriteLine('')
[Console]::Out.WriteLine("$($script:cases) cases run, $($script:failures) failed")
Remove-Item -LiteralPath $T -Recurse -Force -ErrorAction SilentlyContinue
if ($script:failures -eq 0 -and $script:cases -gt 0) { exit 0 } else { exit 1 }
