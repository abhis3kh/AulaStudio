param(
    [ValidateSet('test', 'check', 'fmt', 'clippy', 'release', 'run', 'probe')]
    [string]$Action = 'test',
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') {
    throw 'AULA Studio currently builds for Windows with the MSVC toolchain.'
}

$taskSavedEnvironment = @{}
foreach ($taskName in @('RUSTUP_HOME', 'RUSTUP_TOOLCHAIN', 'CARGO_HOME', 'RUSTC', 'PATH')) {
    $taskSavedEnvironment[$taskName] = [Environment]::GetEnvironmentVariable($taskName, 'Process')
}
$taskLocationPushed = $false

function Invoke-TaskCargo {
    param([string[]]$CargoArguments)
    & $taskCargo @CargoArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo failed with exit code $LASTEXITCODE"
    }
}

try {
    # Prefer a contributor's installed toolchain. The ignored local toolchain is optional.
    $taskInstalledCargo = Get-Command cargo.exe -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -ne $taskInstalledCargo) {
        $taskCargo = $taskInstalledCargo.Source
    } else {
        $taskRustupHome = Join-Path $PSScriptRoot '.tools\rustup'
        $taskPinnedVersion = '1.99.0'
        $taskPinnedBin = Join-Path $taskRustupHome "toolchains\$taskPinnedVersion-x86_64-pc-windows-msvc\bin"
        $taskStableBin = Join-Path $taskRustupHome 'toolchains\stable-x86_64-pc-windows-msvc\bin'
        $taskBin = @($taskPinnedBin, $taskStableBin) |
            Where-Object { Test-Path -LiteralPath (Join-Path $_ 'cargo.exe') -PathType Leaf } |
            Select-Object -First 1
        if ($null -eq $taskBin) {
            throw 'Cargo was not found. Install Rust with the x86_64-pc-windows-msvc toolchain and Microsoft C++ Build Tools, then reopen PowerShell. See README.md.'
        }
        $env:RUSTUP_HOME = $taskRustupHome
        # cargo-fmt and cargo-clippy can invoke rustup themselves. Select the installed
        # local toolchain name so the repository pin does not trigger a second download.
        $env:RUSTUP_TOOLCHAIN = if ($taskBin -eq $taskPinnedBin) { $taskPinnedVersion } else { 'stable' }
        $env:CARGO_HOME = Join-Path $PSScriptRoot '.tools\cargo'
        $env:RUSTC = Join-Path $taskBin 'rustc.exe'
        $env:PATH = $taskBin + [IO.Path]::PathSeparator + $env:PATH
        $taskCargo = Join-Path $taskBin 'cargo.exe'
        $taskRustVersion = & $env:RUSTC --version
        if ($LASTEXITCODE -ne 0 -or $taskRustVersion -notmatch ('^rustc ' + [regex]::Escape($taskPinnedVersion) + '(\s|$)')) {
            throw "The local fallback must contain Rust $taskPinnedVersion. Found: $taskRustVersion. Install the pinned toolchain or use an installed Cargo."
        }
    }

    Push-Location $PSScriptRoot
    $taskLocationPushed = $true
    $taskLocked = @('--locked')
    if ($Offline) {
        $taskLocked += '--offline'
    }
    switch ($Action) {
        'test' { Invoke-TaskCargo (@('test', '--all-targets', '--all-features') + $taskLocked) }
        'check' { Invoke-TaskCargo (@('check', '--all-targets', '--all-features') + $taskLocked) }
        'fmt' { Invoke-TaskCargo @('fmt', '--all', '--check') }
        'clippy' {
            Invoke-TaskCargo (@('clippy', '--all-targets', '--all-features') + $taskLocked + @('--', '-D', 'warnings'))
        }
        'release' {
            Invoke-TaskCargo (@('build', '--release', '--bin', 'aula-studio', '--bin', 'f75-check') + $taskLocked)
        }
        'run' { Invoke-TaskCargo (@('run', '--release', '--bin', 'aula-studio') + $taskLocked) }
        'probe' { Invoke-TaskCargo (@('run', '--bin', 'aula-studio') + $taskLocked + @('--', '--probe')) }
    }
} finally {
    if ($taskLocationPushed) {
        Pop-Location
    }
    foreach ($taskName in $taskSavedEnvironment.Keys) {
        if ($null -eq $taskSavedEnvironment[$taskName]) {
            Remove-Item -LiteralPath ('Env:\' + $taskName) -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($taskName, $taskSavedEnvironment[$taskName], 'Process')
        }
    }
}
