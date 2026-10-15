param(
    [Parameter(Mandatory)]
    [string] $Binary
)

$ErrorActionPreference = 'Stop'
$info = [System.Diagnostics.ProcessStartInfo]::new()
$info.FileName = (Resolve-Path $Binary).Path
$info.UseShellExecute = $false
$info.ArgumentList.Add((Join-Path $PSScriptRoot '..' 'examples' 'welcome.md'))
$process = [System.Diagnostics.Process]::Start($info)

try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) {
            throw "MDV exited during startup with code $($process.ExitCode)."
        }
        if ($IsWindows) {
            $ready = $process.MainWindowHandle -ne 0 -and $process.Responding
        } else {
            $windows = & xdotool search --pid $process.Id --name MDV 2>$null
            $ready = $LASTEXITCODE -eq 0 -and $windows.Count -gt 0
        }
        if ($ready) {
            break
        }
    }
    if (-not $ready) {
        throw 'MDV did not create a responsive native window within 30 seconds.'
    }
    Start-Sleep -Seconds 2
    $process.Refresh()
    if ($process.HasExited) {
        throw 'MDV exited after creating its window.'
    }
    Write-Host 'Native desktop smoke test passed.'
} finally {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id
        $process.WaitForExit()
    }
    $process.Dispose()
}
