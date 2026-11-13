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
$script:lastRes = $null
$script:S = $null

function Test-Case {
  param([string]$Description, [bool]$Condition, [string]$Detail = '')
  $script:cases++
  if ($Condition) {
    [Console]::Out.WriteLine("ok   $Description")
  } else {
    $script:failures++
    [Console]::Out.WriteLine("FAIL $Description")
    if ($Detail) { [Console]::Out.WriteLine("     $Detail") }
    Write-Diagnostic
  }
}

function Write-Diagnostic {
  # After a FAIL: the exit code of the last installer run and a bounded tail of what it wrote (8
  # lines of 200 columns per stream), so a red CI run explains itself. The fakes only carry dummy
  # tokens (ghtok-...); they are masked anyway. The stub logs show what the fakes were asked.
  $r = $script:lastRes
  if ($null -eq $r) { return }
  [Console]::Out.WriteLine("     last run: rc=$($r.Rc)")
  foreach ($name in @('Out', 'Err')) {
    $lines = @(($r[$name] -split "`r?`n") | Where-Object { $_ -ne '' } | Select-Object -Last 8)
    if ($lines.Count -gt 0) {
      [Console]::Out.WriteLine("     $($name.ToLowerInvariant()) (last 8 lines):")
      foreach ($l in $lines) {
        $t = $l -replace 'ghtok-[A-Za-z0-9]*', 'ghtok-***'
        if ($t.Length -gt 200) { $t = $t.Substring(0, 200) }
        [Console]::Out.WriteLine("       | $t")
      }
    }
  }
  foreach ($stubName in @('gh', 'cosign')) {
    $log = Get-StubLog $stubName
    if ($log) {
      $t = ($log -split "`r?`n" | Where-Object { $_ -ne '' } | Select-Object -Last 4) -join ' // '
      if ($t.Length -gt 400) { $t = $t.Substring(0, 400) }
      [Console]::Out.WriteLine("     $stubName.log (last 4): $t")
    }
  }
}

function Write-NoBom {
  param([string]$Path, [string]$Text)
  [IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding($false)))
}

$script:GoodWorkflow = 'IAmSkweetis/branch-tally/.github/workflows/release.yml'
$script:GoodRef = 'refs/heads/main'
$script:GoodIdentity = 'https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main'

function Get-SafePath {
  param([string]$Prefix = '')
  $p = "$($script:S.Stubs);$env:windir\System32;$env:windir"
  if ($Prefix) { $p = "$Prefix;$p" }
  return $p
}

function Write-Stub {
  # A .cmd stub for gh or cosign in the sandbox that logs its arguments to <name>.log. For gh,
  # $Codes is @(auth status rc, attestation verify rc). For cosign, @(rc). The fake attestation was
  # signed by $Signer (gh: workflow path, cosign: certificate identity) on $Ref (gh only). Verify
  # succeeds only when the argv carries the pinned flag with exactly that value, so an attestation
  # "from another workflow or ref", or a missing pin, fails.
  param([string]$Name, [int[]]$Codes, [string]$Signer = '', [string]$Ref = $script:GoodRef)
  $body = "@echo off`r`necho %*>>`"%~dp0$Name.log`"`r`n"
  if ($Name -eq 'gh') {
    if (-not $Signer) { $Signer = $script:GoodWorkflow }
    # No parenthesised blocks: a flat script, so every `exit /b` is a plain top-level exit.
    $body += "if `"%1 %2`"==`"auth status`" exit /b $($Codes[0])`r`n"
    $body += "if not `"%1 %2`"==`"attestation verify`" exit /b 99`r`n"
    $body += "if not $($Codes[1])==0 exit /b $($Codes[1])`r`n"
    # The real interface: gh attestation verify <file-path> --repo <owner/repo>. The archive must
    # exist, and --repo is mandatory (without --repo or --owner the real gh refuses).
    $body += "if not exist `"%3`" exit /b 2`r`n"
    $body += "echo %* | findstr /C:`"--repo IAmSkweetis/branch-tally `" >nul || exit /b 2`r`n"
    $body += "echo %* | findstr /C:`"--signer-workflow $Signer `" >nul || exit /b 1`r`n"
    $body += "echo %* | findstr /C:`"--source-ref $Ref `" >nul || exit /b 1`r`n"
    $body += "echo %* | findstr /C:`"--cert-oidc-issuer https://token.actions.githubusercontent.com`" >nul || exit /b 1`r`n"
    $body += "exit /b 0`r`n"
  } else {
    if (-not $Signer) { $Signer = $script:GoodIdentity }
    # The real interface: cosign verify-blob-attestation --bundle <provenance.sigstore.json>
    # --new-bundle-format --type slsaprovenance1 --certificate-identity <id>
    # --certificate-oidc-issuer <issuer> <blob>. A missing or different piece fails as in cosign.
    $body += "if not `"%1`"==`"verify-blob-attestation`" exit /b 2`r`n"
    $body += "echo %* | findstr /R /C:`"--bundle .*provenance\.sigstore\.json`" >nul || exit /b 2`r`n"
    $body += "echo %* | findstr /C:`"--new-bundle-format`" >nul || exit /b 2`r`n"
    $body += "echo %* | findstr /C:`"--type slsaprovenance1 `" >nul || exit /b 2`r`n"
    $body += "if not $($Codes[0])==0 exit /b $($Codes[0])`r`n"
    $body += "echo %* | findstr /C:`"--certificate-identity $Signer `" >nul || exit /b 1`r`n"
    $body += "echo %* | findstr /C:`"--certificate-oidc-issuer https://token.actions.githubusercontent.com`" >nul || exit /b 1`r`n"
    $body += "exit /b 0`r`n"
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
  $psi.Environment['BRANCH_TALLY_INSTALL_API_URL'] = ([Uri]$script:S.Api).AbsoluteUri
  foreach ($k in @('GH_TOKEN', 'GITHUB_TOKEN')) { $psi.Environment.Remove($k) | Out-Null }
  # A PATH of the sandbox's stubs plus System32 (for cmd.exe), so the host's own gh and cosign can
  # never leak into a case.
  $psi.Environment['PATH'] = Get-SafePath
  foreach ($k in $Env.Keys) { $psi.Environment[$k] = [string]$Env[$k] }
  $p = [System.Diagnostics.Process]::Start($psi)
  $outTask = $p.StandardOutput.ReadToEndAsync()
  $errTask = $p.StandardError.ReadToEndAsync()
  $p.WaitForExit()
  $script:lastRes = @{ Rc = $p.ExitCode; Out = $outTask.Result; Err = $errTask.Result }
  return $script:lastRes
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
    Api     = Join-Path $root 'api'
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
  Write-Api ('{' + "`n  `"url`": `"x`",`n  `"tag_name`": `"$Tag`",`n  `"assets`": []`n}")
}

function Write-Api {
  # What the fake GitHub API answers for releases/latest: exactly this text. Without a call, there
  # is no file, which is an unreachable API.
  param([string]$Body)
  $dir = Join-Path $script:S.Api 'repos\IAmSkweetis\branch-tally\releases'
  New-Item -ItemType Directory -Path $dir -Force | Out-Null
  Write-NoBom (Join-Path $dir 'latest') $Body
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
Write-Stub 'gh' @(0, 0)
$res = Invoke-Installer @('-To', $D)
$ghLog = Get-StubLog 'gh'
Test-Case 'gh pass exits 0' ($res.Rc -eq 0)
Test-Case 'gh pass installs' (Test-Path (Join-Path $D 'branch-tally.exe'))
Test-Case 'gh pass ran attestation verify' ($ghLog -like '*attestation verify*')
Test-Case 'gh pass pins the repository' ($ghLog -like '*--repo IAmSkweetis/branch-tally*')
Test-Case 'gh pass pins the signer workflow' ($ghLog -like "*--signer-workflow $script:GoodWorkflow *")
Test-Case 'gh pass pins the source ref to main' ($ghLog -like '*--source-ref refs/heads/main *')
Test-Case 'gh pass pins the issuer' ($ghLog -like '*--cert-oidc-issuer https://token.actions.githubusercontent.com*')
Test-Case 'gh pass verifies the archive' ($ghLog -like "*branch-tally-v2.0.0-$target.zip*")
Test-Case 'gh pass says provenance was verified' ($res.Out -like '*Provenance verified*')

# 9b. gh logged in and verification fails: hard failure, no cosign fallback.
Initialize-Sandbox 'gh_fail'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
Write-Stub 'gh' @(0, 1)
Write-Stub 'cosign' @(0)
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
Write-Stub 'gh' @(0, 0)
$res = Invoke-Installer @('-To', $D)
Test-Case 'bad checksum exits non-zero' ($res.Rc -ne 0)
Test-Case 'bad checksum never reaches gh attestation' ((Get-StubLog 'gh') -notlike '*attestation*')

# 9d. gh not logged in, cosign verifies: identity and issuer are pinned.
Initialize-Sandbox 'cosign_pass'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
Write-Stub 'gh' @(1, 0)
Write-Stub 'cosign' @(0)
$res = Invoke-Installer @('-To', $D)
$cosignLog = Get-StubLog 'cosign'
Test-Case 'cosign pass exits 0' ($res.Rc -eq 0)
Test-Case 'cosign pass installs' (Test-Path (Join-Path $D 'branch-tally.exe'))
Test-Case 'cosign pass did not run gh attestation verify' ((Get-StubLog 'gh') -notlike '*attestation*')
Test-Case 'cosign pass uses the bundle' ($cosignLog -like '*provenance.sigstore.json*')
Test-Case 'cosign pass pins an exact identity' ($cosignLog -like "*--certificate-identity $script:GoodIdentity *")
Test-Case 'cosign pass uses no identity regexp' ($cosignLog -notlike '*--certificate-identity-regexp*')
Test-Case 'cosign pass pins the issuer' ($cosignLog -like '*https://token.actions.githubusercontent.com*')
Test-Case 'cosign pass says provenance was verified' ($res.Out -like '*Provenance verified*')

# 9e. cosign verification fails: hard failure.
Initialize-Sandbox 'cosign_fail'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
Write-Stub 'cosign' @(1)
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
Write-Stub 'cosign' @(0)
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
Write-Stub 'gh' @(0, 0)
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case '-RequireAttestation with a passing gh exits 0' ($res.Rc -eq 0)

# 9i. gh on PATH but not logged in, no cosign: unavailable.
Initialize-Sandbox 'gh_loggedout'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
Write-Stub 'gh' @(1, 0)
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case 'logged-out gh with no cosign is unavailable (strict fails)' ($res.Rc -ne 0)
Test-Case 'logged-out gh never ran attestation verify' ((Get-StubLog 'gh') -notlike '*attestation*')

# 9j. An attestation from another workflow, or another ref, in this repository must not install.
foreach ($case in @(
    @{ Name = 'gh_other_wf'; Tool = 'gh'; Signer = 'IAmSkweetis/branch-tally/.github/workflows/other.yml'; Ref = $script:GoodRef; Label = 'gh: attestation from another workflow' },
    @{ Name = 'gh_other_ref'; Tool = 'gh'; Signer = $script:GoodWorkflow; Ref = 'refs/heads/some-branch'; Label = 'gh: attestation from another ref' },
    @{ Name = 'cosign_other_wf'; Tool = 'cosign'; Signer = 'https://github.com/IAmSkweetis/branch-tally/.github/workflows/other.yml@refs/heads/main'; Ref = ''; Label = 'cosign: bundle from another workflow' },
    @{ Name = 'cosign_other_ref'; Tool = 'cosign'; Signer = 'https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/feature'; Ref = ''; Label = 'cosign: bundle from another ref' })) {
  Initialize-Sandbox $case.Name
  $R = $script:S.Release; $D = $script:S.Dest
  Build-Release $R 'v2.0.0' $target '2.0.0'
  Build-Latest $R 'v2.0.0'
  if ($case.Tool -eq 'gh') { Write-Stub 'gh' @(0, 0) $case.Signer $case.Ref } else { Write-Stub 'cosign' @(0) $case.Signer }
  $res = Invoke-Installer @('-To', $D)
  Test-Case "$($case.Label) exits non-zero" ($res.Rc -ne 0)
  Test-Case "$($case.Label) installs nothing" (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
}

# 9k. The manual command, in the warning and the strict failure, carries the same pins.
Initialize-Sandbox 'manual_cmd'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
$res = Invoke-Installer @('-To', $D)
Test-Case 'warning command pins the signer workflow' ($res.Err -like "*--signer-workflow $script:GoodWorkflow*")
Test-Case 'warning command pins the source ref' ($res.Err -like '*--source-ref refs/heads/main*')
Test-Case 'warning command pins the issuer' ($res.Err -like '*--cert-oidc-issuer https://token.actions.githubusercontent.com*')
Remove-Item -LiteralPath $D -Recurse -Force
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case 'strict failure command pins the signer workflow' ($res.Err -like "*--signer-workflow $script:GoodWorkflow*")
Test-Case 'strict failure command pins the source ref' ($res.Err -like '*--source-ref refs/heads/main*')
$helpText = [IO.File]::ReadAllText($installer)
Test-Case 'help names the pinned signer workflow and ref' ($helpText -like "*--signer-workflow $script:GoodWorkflow*" -and $helpText -like '*--source-ref refs/heads/main*')
Test-Case 'help names the exact cosign identity' ($helpText -like "*$script:GoodIdentity*")

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

# 13. F-04: "latest" comes from the GitHub API, not from the mirror.
Initialize-Sandbox 'api_wins'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v1.0.0'   # the mirror's latest/ is stale (a rollback)
Write-Api '{"tag_name": "v2.0.0"}'
$res = Invoke-Installer @('-To', $D)
Test-Case 'api: exits 0' ($res.Rc -eq 0)
Test-Case "api: installs the release GitHub names, not the mirror's latest" ($res.Out -like '*branch-tally 2.0.0*')
Test-Case 'api: no fallback warning when the API answered' ($res.Err -notlike '*not confirmed*')

# 13b. Fallback: API unreachable, rate limited or garbage. Warn loudly and use the mirror's latest.
Initialize-Sandbox 'api_down'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Latest $R 'v1.0.0'
Remove-Item -LiteralPath $script:S.Api -Recurse -Force
$res = Invoke-Installer @('-To', $D)
Test-Case 'api down: falls back and exits 0' ($res.Rc -eq 0)
Test-Case "api down: installs the mirror's latest" ($res.Out -like '*branch-tally 1.0.0*')
Test-Case 'api down: warns latest was not confirmed by GitHub' ($res.Err -like '*not confirmed by GitHub*')
Test-Case 'api down: says an older release could be served' ($res.Err -like '*older release*')
Test-Case 'api down: names the mirror as the source' ($res.Err -like '*mirror*')
Test-Case 'api down: prints the -Version route' ($res.Err -like '*-Version*')
Remove-Item -LiteralPath $D -Recurse -Force
Write-Stub 'gh' @(0, 0)
$res = Invoke-Installer @('-To', $D, '-RequireAttestation')
Test-Case 'api down: allowed under -RequireAttestation' ($res.Rc -eq 0)
Test-Case 'api down: still warns under -RequireAttestation' ($res.Err -like '*not confirmed by GitHub*')

Initialize-Sandbox 'api_garbage'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Latest $R 'v1.0.0'
foreach ($garbage in @('<html>rate limited</html>', '{"message":"API rate limit exceeded"}', '', '{"tag_name":"v1.2"}', '{"tag_name":"evil"}',
    '{"tag_name":"v1.0.0;rm -rf /"}', '{"tag_name":"v1.0.0.1"}', '{"tag_name":"v1.0.0\n"}', '{"tag_name":["v1.0.0"]}', '[]')) {
  Write-Api $garbage
  $res = Invoke-Installer @('-To', $D)
  Test-Case "api garbage '$garbage': falls back with a warning" ($res.Err -like '*not confirmed by GitHub*')
  Test-Case "api garbage '$garbage': installs the mirror's latest" ($res.Out -like '*branch-tally 1.0.0*')
}

# 13c. The API base override: https:// or file:// only (any case), printed when set.
Initialize-Sandbox 'api_scheme'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
foreach ($bad in @('http://api.example.test', 'ftp://api.example.test', 'api.example.test')) {
  $res = Invoke-Installer @('-To', $D) @{ BRANCH_TALLY_INSTALL_API_URL = $bad }
  Test-Case "api base '$bad' is rejected" ($res.Rc -ne 0)
  Test-Case "api base '$bad' names the variable" ($res.Err -like '*BRANCH_TALLY_INSTALL_API_URL*')
  Test-Case "api base '$bad' installs nothing" (-not (Test-Path (Join-Path $D 'branch-tally.exe')))
}
$res = Invoke-Installer @('-To', $D)
Test-Case 'a custom api base is printed' ($res.Out -like '*Using release API: file://*')

# 13d. file:// is recognised in any case, in the mirror check and in Get-Remote alike.
$upper = 'FILE://' + ([Uri]$R).AbsoluteUri.Substring(7)
$res = Invoke-Installer @('-To', $D) @{ BRANCH_TALLY_INSTALL_BASE_URL = $upper }
Test-Case 'an uppercase FILE:// mirror installs' ($res.Rc -eq 0)

# 13e. Pinned version: the API is not consulted.
Initialize-Sandbox 'api_pinned'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Release $R 'v2.0.0' $target '2.0.0'
Build-Latest $R 'v2.0.0'
Remove-Item -LiteralPath $script:S.Api -Recurse -Force
$res = Invoke-Installer @('-Version', 'v1.0.0', '-To', $D)
Test-Case 'pinned: exits 0 without the API' ($res.Rc -eq 0)
Test-Case 'pinned: no fallback warning' ($res.Err -notlike '*not confirmed*')

# 13f. A trailing newline is not a valid version (a regexp $ would accept it).
$res = Invoke-Installer @('-To', $D) @{ BRANCH_TALLY_INSTALL_VERSION = "v1.0.0`n" }
Test-Case 'version with a trailing newline is rejected' ($res.Rc -ne 0)

# 13g. A token on a failing lookup is never printed. (The bearer header and the redirect rules apply
# to https:// only. A file:// API sends no request, so they cannot be exercised here: see the
# static checks below and the report.)
Initialize-Sandbox 'api_token_fail'
$R = $script:S.Release; $D = $script:S.Dest
Build-Release $R 'v1.0.0' $target '1.0.0'
Build-Latest $R 'v1.0.0'
Remove-Item -LiteralPath $script:S.Api -Recurse -Force
$res = Invoke-Installer @('-To', $D) @{ GH_TOKEN = 'ghtok-CCC' }
Test-Case 'token on failure: fallback warns' ($res.Err -like '*not confirmed by GitHub*')
Test-Case 'token on failure: never printed' (($res.Out + $res.Err) -notlike '*ghtok-CCC*')

# 13h. Static checks of the redirect handling, since no https server is available to the suite.
$src = [IO.File]::ReadAllText($installer)
Test-Case 'Get-Remote turns automatic redirects off' ($src.Contains('MaximumRedirection = 0;'))
Test-Case 'the latest lookup has a 30 second timeout that every hop uses' ($src.Contains('$json $headers 30') -and $src.Contains('$request[''TimeoutSec''] = $TimeoutSec'))
Test-Case 'Get-Remote has a hop limit' ($src -like '*$maxRedirects*')
Test-Case 'Get-Remote refuses a non-https hop' ($src -like '*refusing to follow a non-https redirect*')
# .Contains, not -like: a [ in a -like pattern is a wildcard character class.
Test-Case 'Get-Remote compares the file scheme case-insensitively' ($src.Contains("StartsWith('file://', [System.StringComparison]::OrdinalIgnoreCase)"))
Test-Case 'the version check uses \z, not $' ($src.Contains('[0-9]+\.[0-9]+\.[0-9]+\z') -and -not $src.Contains('[0-9]+\.[0-9]+\.[0-9]+$'''))

# 12. The help names the new parameter.
$helpText = [IO.File]::ReadAllText($installer)
Test-Case 'help documents -RequireAttestation' ($helpText -like '*-RequireAttestation*' -and $helpText -like '*BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION*')
Test-Case 'help calls the checksum an integrity check' ($helpText -like '*integrity check*')
Test-Case 'help documents the API base variable' ($helpText -like '*BRANCH_TALLY_INSTALL_API_URL*')
Test-Case 'help mentions GH_TOKEN' ($helpText -like '*GH_TOKEN*')
Test-Case 'help explains the mirror fallback' ($helpText -like '*not confirmed by GitHub*')

[Console]::Out.WriteLine('')
[Console]::Out.WriteLine("$($script:cases) cases run, $($script:failures) failed")
Remove-Item -LiteralPath $T -Recurse -Force -ErrorAction SilentlyContinue
if ($script:failures -eq 0 -and $script:cases -gt 0) { exit 0 } else { exit 1 }
