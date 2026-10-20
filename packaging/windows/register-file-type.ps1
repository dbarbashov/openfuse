param([Parameter(Mandatory=$true)][ValidateSet('Install','Uninstall')][string]$Action)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$exe = Join-Path $root 'bin\openfuse-desktop.exe'
$class = 'HKCU:\Software\Classes\OpenFUSE.Preset'
$openWith = 'HKCU:\Software\Classes\.fuse\OpenWithProgids'
if (Test-Path $class) {
    $owner = (Get-ItemProperty $class).OpenFUSERoot
    if ($owner -ne $root) { throw 'Another installation owns this association; unregister it first.' }
}
if ($Action -eq 'Install') {
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Packaged executable is missing.' }
    New-Item $class -Force | Out-Null
    Set-Item $class -Value 'Legacy FUSE preset'
    New-ItemProperty $class -Name 'OpenFUSERoot' -Value $root -Force | Out-Null
    New-Item "$class\shell\open\command" -Force | Out-Null
    Set-Item "$class\shell\open\command" -Value ('"' + $exe + '" "%1"')
    New-Item $openWith -Force | Out-Null
    New-ItemProperty $openWith -Name 'OpenFUSE.Preset' -PropertyType String -Value '' -Force | Out-Null
    Write-Output 'OpenFUSE is available in Open With. Your default application was not changed.'
} elseif (Test-Path $class) {
    Remove-Item $class -Recurse
    if (Test-Path $openWith) {
        Remove-ItemProperty $openWith -Name 'OpenFUSE.Preset' -ErrorAction SilentlyContinue
    }
    Write-Output 'Removed this installation from Open With.'
}
