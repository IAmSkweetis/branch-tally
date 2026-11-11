# PSScriptAnalyzer over the given files. Warnings fail the run, not only errors.
# Usage: pwsh -NoProfile -File scripts/lint-install.ps1 <file>...
param(
    [Parameter(Mandatory, ValueFromRemainingArguments)]
    [string[]] $Path
)

$ErrorActionPreference = 'Stop'

if (-not (Get-Module -ListAvailable -Name PSScriptAnalyzer)) {
    [Console]::Error.WriteLine('error: PSScriptAnalyzer not found; install it: Install-Module PSScriptAnalyzer -Scope CurrentUser')
    exit 1
}

$findings = @($Path | ForEach-Object { Invoke-ScriptAnalyzer -Path $_ -Severity Warning, Error })
if ($findings.Count -gt 0) {
    $findings | Format-Table -AutoSize -Wrap ScriptName, Line, Severity, RuleName, Message | Out-String -Width 200 | Write-Output
    [Console]::Error.WriteLine("error: PSScriptAnalyzer reported $($findings.Count) finding(s)")
    exit 1
}
Write-Output "PSScriptAnalyzer: $($Path.Count) file(s) clean"
