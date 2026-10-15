param(
    [string] $Document,
    [string] $Output = (Join-Path $PSScriptRoot '..' 'target' 'scroll-diagnostics'),
    [double] $MaximumFrameP95Ms = 50,
    [ValidateSet('software', 'gpu')]
    [string] $Renderer = 'software'
)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows -and $env:MDV_SCROLL_XVFB_CHILD -ne '1') {
    $env:MDV_SCROLL_XVFB_CHILD = '1'
    try {
        $arguments = @('-File', $PSCommandPath, '-Output', $Output, '-MaximumFrameP95Ms', $MaximumFrameP95Ms, '-Renderer', $Renderer)
        if ($Document) { $arguments += @('-Document', $Document) }
        & xvfb-run --auto-servernum pwsh @arguments
        if ($LASTEXITCODE -ne 0) { throw 'The isolated native scrolling check failed.' }
    } finally {
        Remove-Item Env:MDV_SCROLL_XVFB_CHILD
    }
    return
}
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$fixture = $null
$process = $null
try {
    if ($Renderer -eq 'gpu' -and -not $IsWindows) {
        throw 'The GPU comparison diagnostic currently targets the Windows rendering path.'
    }
    if (-not $Document) {
        $fixture = Join-Path ([IO.Path]::GetTempPath()) "mdv-scroll-check-$([Guid]::NewGuid().ToString('N'))"
        New-Item -ItemType Directory -Path $fixture | Out-Null
        $Document = Join-Path $fixture 'scroll.md'
        $markdown = [Text.StringBuilder]::new()
        foreach ($section in 1..8) {
            [void] $markdown.AppendLine("## Section $section`n")
            [void] $markdown.AppendLine('A paragraph with **bold text**, *emphasis*, and `inline code`. The reading pane should scroll smoothly while preserving the text layout.')
            [void] $markdown.AppendLine("`n| Topic | Description | Observation | Next step |`n| --- | --- | --- | --- |")
            foreach ($row in 1..6) {
                [void] $markdown.AppendLine("| Row $row | A longer description that wraps naturally inside the table column. | Compare the visible result with the expected result before changing anything. | Read the next section and record a clear conclusion. |")
            }
            [void] $markdown.AppendLine("`n1. First ordered item`n2. Second ordered item`n3. Third ordered item`n")
            [void] $markdown.AppendLine('```rust')
            [void] $markdown.AppendLine('let answer = 42;')
            [void] $markdown.AppendLine('println!("{answer}");')
            [void] $markdown.AppendLine('```')
            [void] $markdown.AppendLine("`n![A local image](mark.svg)`n")
        }
        [IO.File]::WriteAllText($Document, $markdown.ToString())
        Copy-Item (Join-Path $root 'examples' 'mark.svg') (Join-Path $fixture 'mark.svg')
    }
    New-Item -ItemType Directory -Path $Output -Force | Out-Null
    $Output = (Resolve-Path $Output).Path
    $artifacts = & cargo test --release --locked --no-run --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot build the native scrolling test.' }
    $executable = $artifacts | ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'mdv' -and $_.profile.test -and $_.executable } |
        Select-Object -Last 1 -ExpandProperty executable
    if (-not $executable) { throw 'Cargo did not report a native test executable.' }

    if ($IsWindows) {
        Add-Type @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class MdvScrollingCheck {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", SetLastError = true)] static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    public static IntPtr Find(int process) {
        var window = FindWindow(null, "MDV local scroll diagnosis");
        GetWindowThreadProcessId(window, out var owner);
        return owner == process ? window : IntPtr.Zero;
    }
    public static void Wheel(IntPtr window, int amount) {
        if (!PostMessage(window, 0x020A, new IntPtr(amount << 16), IntPtr.Zero))
            throw new Win32Exception();
    }
}
'@
    }
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $executable
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $test = if ($Renderer -eq 'gpu') { 'native_scroll_diagnostic' } else { 'native_software_scroll_diagnostic' }
    $info.ArgumentList.Add($test)
    $info.ArgumentList.Add('--ignored')
    $info.ArgumentList.Add('--nocapture')
    $info.Environment['MDV_NATIVE_DOCUMENT'] = (Resolve-Path $Document).Path
    $info.Environment['MDV_NATIVE_PROFILE_DIR'] = $Output
    $info.Environment['MDV_NATIVE_PROFILE_SECONDS'] = '15'
    $info.Environment['MDV_NATIVE_VIRTUAL_HOVER'] = '1'
    $process = [Diagnostics.Process]::Start($info)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $window = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 100
        $process.Refresh()
        if ($process.HasExited) { throw "The native test exited during startup.`n$($stdout.Result)`n$($stderr.Result)" }
        if ($IsWindows) {
            $window = [MdvScrollingCheck]::Find($process.Id)
        } else {
            $found = & xdotool search --pid $process.Id --name 'MDV local scroll diagnosis' 2>$null
            if ($LASTEXITCODE -eq 0) { $window = @($found)[0] }
        }
    } while (-not $window -and [DateTime]::UtcNow -lt $deadline)
    if (-not $window) { throw 'The native test window was not found.' }
    if (-not $IsWindows) {
        & xdotool mousemove --window $window 700 400
        if ($LASTEXITCODE -ne 0) { throw 'Cannot position the pointer in the isolated test display.' }
    }
    Start-Sleep -Seconds 2
    foreach ($step in 0..79) {
        if ($process.HasExited) { break }
        if ($IsWindows) {
            $amount = if ($step -lt 60) { -360 } else { 360 }
            [MdvScrollingCheck]::Wheel($window, $amount)
        } else {
            $button = if ($step -lt 60) { 5 } else { 4 }
            & xdotool click --repeat 3 --delay 0 $button
            if ($LASTEXITCODE -ne 0) { throw 'Cannot send native scroll input.' }
        }
        Start-Sleep -Milliseconds 90
    }
    if (-not $process.WaitForExit(25000)) { throw 'The native scrolling test did not exit.' }
    if ($process.ExitCode -ne 0) { throw "The native test failed.`n$($stdout.Result)`n$($stderr.Result)" }
    $rows = @(Import-Csv (Join-Path $Output 'frames.csv'))
    $received = ($rows | ForEach-Object { [int]$_.wheel_events } | Measure-Object -Maximum).Maximum
    Write-Host "Captured $($rows.Count) native frames and $received wheel events."
    $factor = if ($IsWindows) { 1 } else { 3 }
    $active = @($rows | Where-Object {
        [int]$_.wheel_events -ge 5 * $factor -and [int]$_.wheel_events -le 70 * $factor
    })
    if ($active.Count -lt 20) { throw "Too few active scrolling frames: $($active.Count).`n$($stdout.Result)`n$($stderr.Result)" }
    $maxOffset = ($rows | ForEach-Object { [double]$_.offset } | Measure-Object -Maximum).Maximum
    $lastOffset = [double]$rows[-1].offset
    if ($maxOffset -lt 200 -or $lastOffset -gt $maxOffset - 100) {
        throw 'Native wheel input did not move the document in both directions.'
    }
    $intervals = @($active | ForEach-Object { [double]$_.interval_ms } | Sort-Object)
    $render = @($active | ForEach-Object { [double]$_.cpu_ms } | Sort-Object)
    $middle = [int][Math]::Floor($active.Count / 2)
    $p95 = [int][Math]::Floor($active.Count * 0.95)
    Write-Host "Native scrolling: $($active.Count) frames, offset 0 -> $maxOffset -> $lastOffset"
    Write-Host "Frame intervals: median $($intervals[$middle]) ms, p95 $($intervals[$p95]) ms"
    Write-Host "Render + present: median $($render[$middle]) ms, p95 $($render[$p95]) ms"
    if ($intervals[$p95] -gt $MaximumFrameP95Ms) {
        throw "Native frame pacing exceeded $MaximumFrameP95Ms ms at the 95th percentile."
    }
} finally {
    if ($process) {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id }
        $process.Dispose()
    }
    if ($fixture) {
        [IO.File]::Delete((Join-Path $fixture 'scroll.md'))
        [IO.File]::Delete((Join-Path $fixture 'mark.svg'))
        [IO.Directory]::Delete($fixture)
    }
}
