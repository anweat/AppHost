#Requires -Version 7.0
param([Parameter(Mandatory=$true)][string]$Exe)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This smoke test requires Windows.' }
$sourceExe = (Resolve-Path -LiteralPath $Exe).Path
$root = Join-Path ([System.IO.Path]::GetTempPath()) ('apphost-smoke-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
try {
    # Portable install in a disposable directory; never changes PATH or login tasks.
    $installed = Join-Path $root 'apphost.exe'
    Copy-Item -LiteralPath $sourceExe -Destination $installed
    $data = Join-Path $root 'data'
    function Invoke-AppHost([string[]]$Arguments, [bool]$ExpectSuccess = $true) {
        $info = [System.Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $installed
        $info.UseShellExecute = $false
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $info.Environment.Remove('APPHOST_ACTOR') | Out-Null
        foreach ($arg in @('--data-dir', $data) + $Arguments) { $info.ArgumentList.Add($arg) }
        $process = [System.Diagnostics.Process]::Start($info)
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(15000)) { $process.Kill($true); throw 'CLI deadline exceeded' }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($ExpectSuccess) {
            if ($process.ExitCode -ne 0 -or $stderr.Length -ne 0) { throw "CLI failed: $stderr" }
            return ($stdout | ConvertFrom-Json)
        }
        if ($process.ExitCode -eq 0 -or $stdout.Length -ne 0) { throw 'Expected a JSON error' }
        return ($stderr | ConvertFrom-Json)
    }
    $guide = Invoke-AppHost @('agent-guide')
    if (-not $guide.version -or -not $guide.manifest) { throw 'Embedded guide missing' }
    $policy = Invoke-AppHost @('log-retention','show')
    if ($policy.days -ne 7 -or (Test-Path -LiteralPath $data)) { throw 'Read unexpectedly created storage' }
    $project = Join-Path $root 'project with spaces'
    New-Item -ItemType Directory -Path $project | Out-Null
    $projectId = [guid]::NewGuid().ToString()
    $manifest = @"
schema_version = 1
project_id = "$projectId"
name = "Windows smoke"
[[services]]
name = "fixture"
command = ["unused-command"]
"@
    Set-Content -LiteralPath (Join-Path $project 'apphost.toml') -Value $manifest -Encoding utf8NoBOM
    $null = Invoke-AppHost @('validate',$project)
    $draft = Join-Path $root 'draft.json'
    $null = Invoke-AppHost @('scan',$project,'--output',$draft)
    $receipt = @(Invoke-AppHost @('import',$draft,'--yes'))
    $id = $receipt[0].installation_id
    if (-not $id) { throw 'Import did not return installation identity' }
    $safe = @(Invoke-AppHost @('list','--redacted'))
    if ($safe.Count -ne 1 -or $safe[0].installation_id -ne $id) { throw 'Registration mismatch' }
    $null = Invoke-AppHost @('label',$id,'Renamed smoke')
    $null = Invoke-AppHost @('log-retention','set','--days','3','--max-mib','32')
    $null = Invoke-AppHost @('icon',$id)
    $errorResult = Invoke-AppHost @('start',$id) $false
    if ($errorResult.error.code -ne 'UNSUPPORTED_PLATFORM') { throw 'Runtime support boundary changed' }
    $null = Invoke-AppHost @('detach',$id)
    if (-not (Test-Path -LiteralPath (Join-Path $project 'apphost.toml'))) { throw 'Detach removed project files' }
    [pscustomobject]@{ result='passed'; version=$guide.version; platform='Windows'; scope='portable CLI installation, registration and read-only delivery contracts'; runtime='not supported until G3' } | ConvertTo-Json
} finally {
    # Only the uniquely generated smoke directory is removed.
    Remove-Item -LiteralPath $root -Recurse -Force
}
