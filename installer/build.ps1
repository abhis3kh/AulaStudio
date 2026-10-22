param([Parameter(Mandatory)][string]$BinaryDirectory)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$binaryRoot = (Resolve-Path -LiteralPath $BinaryDirectory).Path
$compiler = Join-Path $repo '.tools\nsis-3.12\makensis.exe'
if ($env:NSIS_HOME) {
    $compiler = Join-Path $env:NSIS_HOME 'makensis.exe'
} elseif (-not (Test-Path -LiteralPath $compiler)) {
    $installed = Get-Command makensis.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($installed) { $compiler = $installed.Source }
}
if (-not (Test-Path -LiteralPath $compiler)) {
    throw 'Install NSIS 3.12 or newer and set NSIS_HOME to its folder, or extract NSIS 3.12 into .tools. See README.md.'
}
$version = [regex]::Match((Get-Content (Join-Path $repo 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"').Groups[1].Value
if (-not $version) { throw 'Cargo package version must be major.minor.patch.' }
$generated = Join-Path (Split-Path $binaryRoot -Parent) 'installer'
New-Item -ItemType Directory -Path $generated -Force | Out-Null

# Exact file lists keep uninstallation from deleting unrelated user files.
$payload = @(
    @{ Source = Join-Path $binaryRoot 'aula-studio.exe'; Dest = 'aula-studio.exe' },
    @{ Source = Join-Path $binaryRoot 'f75-check.exe'; Dest = 'f75-check.exe' }
)
foreach ($name in @('LICENSE', 'README.md', 'THIRD_PARTY_NOTICES.md')) {
    $payload += @{ Source = Join-Path $repo $name; Dest = $name }
}
foreach ($file in Get-ChildItem (Join-Path $repo 'licenses') -Recurse -File | Sort-Object FullName) {
    $payload += @{ Source = $file.FullName; Dest = [IO.Path]::GetRelativePath($repo, $file.FullName) }
}
function Escape-Nsis([string]$Value) {
    $Value.Replace('$', '$$').Replace('"', '$\"')
}
$install = [Collections.Generic.List[string]]::new()
$uninstall = [Collections.Generic.List[string]]::new()
$directories = [Collections.Generic.HashSet[string]]::new()
foreach ($item in $payload) {
    if (-not (Test-Path -LiteralPath $item.Source -PathType Leaf)) { throw "Missing installer file: $($item.Source)" }
    $directory = Split-Path $item.Dest -Parent
    $destination = if ($directory) { '$INSTDIR\' + (Escape-Nsis $directory) } else { '$INSTDIR' }
    $install.Add('SetOutPath "' + $destination + '"')
    $install.Add('File /oname=' + (Split-Path $item.Dest -Leaf) + ' "' + (Escape-Nsis $item.Source) + '"')
    $uninstall.Add('Delete "$INSTDIR\' + (Escape-Nsis $item.Dest) + '"')
    while ($directory) {
        $null = $directories.Add($directory)
        $directory = Split-Path $directory -Parent
    }
}
foreach ($directory in $directories | Sort-Object Length -Descending) {
    $uninstall.Add('RMDir "$INSTDIR\' + (Escape-Nsis $directory) + '"')
}
$installFile = Join-Path $generated 'install-files.nsh'
$uninstallFile = Join-Path $generated 'uninstall-files.nsh'
[IO.File]::WriteAllLines($installFile, $install, [Text.UTF8Encoding]::new($true))
[IO.File]::WriteAllLines($uninstallFile, $uninstall, [Text.UTF8Encoding]::new($true))
$output = Join-Path $PSScriptRoot 'AulaStudio-Setup.exe'
& $compiler /V2 /WX "/DAPP_VERSION=$version" "/DREPO_ROOT=$repo" "/DINSTALL_FILES=$installFile" "/DUNINSTALL_FILES=$uninstallFile" "/DOUTPUT_FILE=$output" (Join-Path $PSScriptRoot 'setup.nsi')
if ($LASTEXITCODE -ne 0) { throw "Installer compilation failed: $LASTEXITCODE" }
Get-Item -LiteralPath $output | Select-Object FullName, Length
