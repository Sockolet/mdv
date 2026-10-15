param(
    [Parameter(Mandatory)]
    [string] $Binary,
    [switch] $Scroll
)

$ErrorActionPreference = 'Stop'
$process = $null
$temporaryFolder = $null

if ($IsWindows) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class MdvWindowLookup {
    delegate bool WindowCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(WindowCallback callback, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder text, int length);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wparam, IntPtr lparam, uint flags, uint timeout, out IntPtr result);
    public static IntPtr Find(int process) {
        var found = IntPtr.Zero;
        EnumWindows((window, _) => {
            GetWindowThreadProcessId(window, out var owner);
            if (owner != process) return true;
            var title = new StringBuilder(256);
            GetWindowText(window, title, title.Capacity);
            if (!title.ToString().StartsWith("MDV")) return true;
            found = window;
            return false;
        }, IntPtr.Zero);
        return found;
    }
    public static bool Responsive(IntPtr window) {
        return SendMessageTimeout(window, 0, IntPtr.Zero, IntPtr.Zero, 2, 1000, out _) != IntPtr.Zero;
    }
    public static string Title(IntPtr window) {
        var title = new StringBuilder(256);
        GetWindowText(window, title, title.Capacity);
        return title.ToString();
    }
}
'@
}

try {
    $document = Join-Path $PSScriptRoot '..' 'examples' 'welcome.md'
    if ($Scroll) {
        $temporaryFolder = Join-Path ([IO.Path]::GetTempPath()) "mdv-scroll-$([Guid]::NewGuid().ToString('N'))"
        New-Item -ItemType Directory -Path $temporaryFolder | Out-Null
        $document = Join-Path $temporaryFolder 'scroll.md'
        $markdown = [Text.StringBuilder]::new()
        foreach ($section in 1..1000) {
            [void] $markdown.AppendLine("## Section $section`n")
            [void] $markdown.AppendLine('A paragraph with **bold text**, *emphasis*, and [a link](other.md).')
            [void] $markdown.AppendLine("`n- First item`n- Second item`n- Third item`n")
            [void] $markdown.AppendLine('```rust')
            [void] $markdown.AppendLine('let answer = 42;')
            [void] $markdown.AppendLine('```')
            [void] $markdown.AppendLine("`n| Name | Value |`n| --- | --- |`n| Example | $section |`n")
        }
        [IO.File]::WriteAllText($document, $markdown.ToString())
    }
    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = (Resolve-Path $Binary).Path
    $info.UseShellExecute = $false
    $info.ArgumentList.Add($document)
    $process = [System.Diagnostics.Process]::Start($info)
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) {
            throw "MDV exited during startup with code $($process.ExitCode)."
        }
        if ($IsWindows) {
            $window = [MdvWindowLookup]::Find($process.Id)
            $ready = $window -ne 0 -and [MdvWindowLookup]::Responsive($window)
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
    if ($IsWindows) { Write-Host "Opened: $([MdvWindowLookup]::Title($window))" }
    if ($Scroll) {
        if ($IsWindows) {
            Add-Type @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Threading;
public static class MdvScrollInput {
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
    [DllImport("user32.dll", SetLastError = true)] static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError = true)] static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll", SetLastError = true)] static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    public static void Scroll(IntPtr window) {
        if (!GetClientRect(window, out var rect)) throw new Win32Exception();
        var point = new Point { X = rect.Right * 3 / 4, Y = rect.Bottom / 2 };
        if (!PostMessage(window, 0x0200, IntPtr.Zero, new IntPtr((point.Y << 16) | (point.X & 0xffff))))
            throw new Win32Exception();
        if (!ClientToScreen(window, ref point)) throw new Win32Exception();
        for (int frame = 0; frame < 90; frame++) {
            int amount = frame < 60 ? -120 : 120;
            if (!PostMessage(window, 0x020A, new IntPtr(amount << 16), new IntPtr((point.Y << 16) | (point.X & 0xffff))))
                throw new Win32Exception();
            Thread.Sleep(8);
        }
    }
}
'@
            [MdvScrollInput]::Scroll($window)
        } else {
            $window = @($windows)[0]
            & xdotool mousemove --window $window 700 400
            if ($LASTEXITCODE -ne 0) { throw 'Cannot position the test pointer.' }
            & xdotool click --window $window --repeat 60 --delay 8 5
            if ($LASTEXITCODE -ne 0) { throw 'Cannot send downward scroll input.' }
            & xdotool click --window $window --repeat 30 --delay 8 4
            if ($LASTEXITCODE -ne 0) { throw 'Cannot send upward scroll input.' }
        }
    }
    Start-Sleep -Seconds 2
    $process.Refresh()
    if ($process.HasExited) {
        throw 'MDV exited after creating its window.'
    }
    if ($IsWindows -and -not [MdvWindowLookup]::Responsive($window)) {
        throw 'MDV stopped responding while scrolling.'
    }
    Write-Host 'Native desktop startup smoke test passed.'
} finally {
    if ($process) {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id
            $process.WaitForExit()
        }
        $process.Dispose()
    }
    if ($temporaryFolder) {
        [IO.File]::Delete($document)
        [IO.Directory]::Delete($temporaryFolder)
    }
}
