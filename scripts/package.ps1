param(
    [Parameter(Mandatory)]
    [ValidateSet('x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu')]
    [string] $Target
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dist = Join-Path $root 'dist'
$name = "mdv-$Target"
$stage = Join-Path $dist $name
if (Test-Path $stage) {
    throw "Package staging folder already exists: $stage"
}
New-Item -ItemType Directory -Path $stage -Force | Out-Null

$executable = if ($Target -like '*windows*') { 'mdv.exe' } else { 'mdv' }
$binary = Join-Path $root 'target' $Target 'release' $executable
Copy-Item $binary (Join-Path $stage $executable)
Copy-Item (Join-Path $root 'LICENSE') $stage
Copy-Item (Join-Path $root 'README.md') $stage
Copy-Item (Join-Path $root 'examples') $stage -Recurse

$metadataJson = & cargo metadata --locked --format-version 1 --filter-platform $Target
if ($LASTEXITCODE -ne 0) {
    throw 'Cannot read dependency metadata for the license report.'
}
$metadata = $metadataJson | ConvertFrom-Json
$notices = [System.Text.StringBuilder]::new()
[void] $notices.AppendLine('MDV third-party dependency and bundled font licenses')
[void] $notices.AppendLine('Includes resolved transitive and development dependencies.')
foreach ($package in ($metadata.packages | Sort-Object name, version)) {
    if ($package.name -eq 'mdv') {
        continue
    }
    [void] $notices.AppendLine("`n$('=' * 72)")
    [void] $notices.AppendLine("$($package.name) $($package.version)")
    [void] $notices.AppendLine("License: $($package.license)")
    [void] $notices.AppendLine("Source: $($package.repository)")
    $packageRoot = Split-Path $package.manifest_path
    $licenseFiles = @(Get-ChildItem $packageRoot -File -Recurse -Depth 4 |
        Where-Object {
            $_.Name -match '(?i)(licen[cs]e|copying|copyright|notice|OFL\.txt|UFL\.txt|Hack-Regular\.txt)' -and
            $_.Extension -in @('', '.txt', '.md', '.rst', '.html')
        })
    if ($package.license_file) {
        $licenseFiles += Get-Item (Join-Path $packageRoot $package.license_file)
    }
    foreach ($file in ($licenseFiles | Sort-Object FullName -Unique)) {
        [void] $notices.AppendLine("`n--- $([System.IO.Path]::GetRelativePath($packageRoot, $file.FullName)) ---")
        [void] $notices.AppendLine([System.IO.File]::ReadAllText($file.FullName))
    }
}
[System.IO.File]::WriteAllText((Join-Path $stage 'THIRD-PARTY-LICENSES.txt'), $notices.ToString())

if ($Target -like '*windows*') {
    $archive = Join-Path $dist "$name.zip"
    Compress-Archive -Path $stage -DestinationPath $archive -Force
} else {
    $archive = Join-Path $dist "$name.tar.gz"
    & tar -czf $archive -C $dist $name
    if ($LASTEXITCODE -ne 0) {
        throw 'Cannot create the Linux archive.'
    }
}
$hash = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText("$archive.sha256", "$hash  $([System.IO.Path]::GetFileName($archive))`n")
Write-Host "Created $archive"
