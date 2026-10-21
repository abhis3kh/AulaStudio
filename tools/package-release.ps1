param(
    [string]$BinaryDirectory = (Join-Path $PSScriptRoot '..\target\release'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\dist\releases')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$taskProject = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskBinaryDirectory = [IO.Path]::GetFullPath($BinaryDirectory)
$taskOutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$taskManifest = Get-Content -LiteralPath (Join-Path $taskProject 'Cargo.toml') -Raw
$taskPackage = [regex]::Match($taskManifest, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
$taskVersionMatch = [regex]::Match($taskPackage.Groups[1].Value, '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)"\s*$')
if (-not $taskVersionMatch.Success) {
    throw 'Cannot read a safe package version from Cargo.toml.'
}
$taskVersion = $taskVersionMatch.Groups[1].Value
$taskFiles = @(
    @{ Source = (Join-Path $taskBinaryDirectory 'aula-studio.exe'); Name = 'AulaStudio.exe' },
    @{ Source = (Join-Path $taskBinaryDirectory 'f75-check.exe'); Name = 'f75-check.exe' },
    @{ Source = (Join-Path $taskProject 'LICENSE'); Name = 'LICENSE' },
    @{ Source = (Join-Path $taskProject 'README.md'); Name = 'README.md' },
    @{ Source = (Join-Path $taskProject 'THIRD_PARTY_NOTICES.md'); Name = 'THIRD_PARTY_NOTICES.md' },
    @{ Source = (Join-Path $taskProject 'CONTRIBUTING.md'); Name = 'CONTRIBUTING.md' },
    @{ Source = (Join-Path $taskProject 'docs\RELIABILITY.md'); Name = 'docs\RELIABILITY.md' },
    @{ Source = (Join-Path $taskProject 'docs\PROVENANCE.md'); Name = 'docs\PROVENANCE.md' },
    @{ Source = (Join-Path $taskProject 'docs\VALIDATION.md'); Name = 'docs\VALIDATION.md' },
    @{ Source = (Join-Path $taskProject 'docs\PUBLISHING.md'); Name = 'docs\PUBLISHING.md' },
    @{ Source = (Join-Path $taskProject 'docs\screenshots\music-palettes.png'); Name = 'docs\screenshots\music-palettes.png' }
)
foreach ($taskFile in $taskFiles) {
    if (-not (Test-Path -LiteralPath $taskFile.Source -PathType Leaf)) {
        throw "Required release file missing: $($taskFile.Source). Build both release binaries and generate dependency notices first."
    }
    if ((Get-Item -LiteralPath $taskFile.Source).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Release inputs must be regular files: $($taskFile.Source)"
    }
}

# Prevent a mislabeled release when a different compiler target was selected.
foreach ($taskExecutable in $taskFiles | Where-Object { $_.Name.EndsWith('.exe') }) {
    $taskStream = [IO.File]::OpenRead($taskExecutable.Source)
    $taskReader = New-Object IO.BinaryReader($taskStream)
    try {
        if ($taskReader.ReadUInt16() -ne 0x5a4d) { throw 'Missing DOS signature.' }
        $taskStream.Position = 0x3c
        $taskPeOffset = $taskReader.ReadUInt32()
        if ($taskPeOffset -gt $taskStream.Length - 6) { throw 'Invalid PE header offset.' }
        $taskStream.Position = $taskPeOffset
        if ($taskReader.ReadUInt32() -ne 0x4550 -or $taskReader.ReadUInt16() -ne 0x8664) {
            throw 'Expected a Windows x86_64 executable.'
        }
    } catch {
        throw "Invalid release binary $($taskExecutable.Name): $_"
    } finally {
        $taskReader.Dispose()
        $taskStream.Dispose()
    }
}

$taskLicenseRoot = Join-Path $taskProject 'licenses'
if (-not (Test-Path -LiteralPath $taskLicenseRoot -PathType Container)) {
    throw 'Missing licenses directory. Generate the third-party notices first.'
}
$taskLicenseItems = @(Get-Item -LiteralPath $taskLicenseRoot) + @(Get-ChildItem -LiteralPath $taskLicenseRoot -Recurse -Force)
if ($taskLicenseItems | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }) {
    throw 'The licenses directory must not contain symbolic links or junctions.'
}
$taskLicenseFiles = @($taskLicenseItems | Where-Object { -not $_.PSIsContainer })
if ($taskLicenseFiles.Count -eq 0) { throw 'The licenses directory is empty.' }
foreach ($taskFile in $taskLicenseFiles) {
    $taskRelative = $taskFile.FullName.Substring($taskLicenseRoot.Length + 1)
    $taskRustCopyright = $taskRelative -match '^rust-[0-9]+\.[0-9]+\.[0-9]+[\\/]COPYRIGHT-library\.html$'
    if ($taskFile.Extension -notin @('.txt', '.md') -and $taskRelative -ne 'manifest.json' -and -not $taskRustCopyright) {
        throw "Unexpected file in licenses: $taskRelative"
    }
}

$taskTargetRoot = [IO.Path]::GetFullPath((Join-Path $taskProject 'target'))
[void][IO.Directory]::CreateDirectory($taskTargetRoot)
$taskStage = Join-Path $taskTargetRoot ('.package-stage-' + [Guid]::NewGuid().ToString('N'))
$taskTemporaryZip = Join-Path $taskTargetRoot ('.package-' + [Guid]::NewGuid().ToString('N') + '.zip')
$taskZipName = "AulaStudio-$taskVersion-windows-x86_64.zip"
$taskZip = Join-Path $taskOutputDirectory $taskZipName
try {
    [void][IO.Directory]::CreateDirectory($taskStage)
    foreach ($taskFile in $taskFiles) {
        $taskDestination = Join-Path $taskStage $taskFile.Name
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($taskDestination))
        Copy-Item -LiteralPath $taskFile.Source -Destination $taskDestination
    }
    foreach ($taskFile in $taskLicenseFiles) {
        $taskRelative = $taskFile.FullName.Substring($taskLicenseRoot.Length + 1)
        $taskDestination = Join-Path (Join-Path $taskStage 'licenses') $taskRelative
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($taskDestination))
        Copy-Item -LiteralPath $taskFile.FullName -Destination $taskDestination
    }
    [IO.Compression.ZipFile]::CreateFromDirectory($taskStage, $taskTemporaryZip, [IO.Compression.CompressionLevel]::Optimal, $false)
    [void][IO.Directory]::CreateDirectory($taskOutputDirectory)
    Move-Item -LiteralPath $taskTemporaryZip -Destination $taskZip -Force
    $taskHash = (Get-FileHash -LiteralPath $taskZip -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($taskZip + '.sha256', "$taskHash  $taskZipName`n", [Text.Encoding]::ASCII)
    Write-Output "Packaged $taskZip"
    Write-Output "SHA-256 $taskHash"
} finally {
    # Only remove the exact random staging directory created inside this project's target.
    $taskResolvedStage = [IO.Path]::GetFullPath($taskStage)
    if ([IO.Path]::GetDirectoryName($taskResolvedStage) -ne $taskTargetRoot -or
        -not [IO.Path]::GetFileName($taskResolvedStage).StartsWith('.package-stage-')) {
        throw 'Refusing to clean a staging path outside the project target directory.'
    }
    if (Test-Path -LiteralPath $taskResolvedStage) {
        Remove-Item -LiteralPath $taskResolvedStage -Recurse -Force
    }
    if (Test-Path -LiteralPath $taskTemporaryZip) {
        Remove-Item -LiteralPath $taskTemporaryZip -Force
    }
}
