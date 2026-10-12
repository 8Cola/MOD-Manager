$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$cachedModules = [IO.Path]::GetFullPath((Join-Path $projectRoot '_local/node_modules'))
$normalModules = [IO.Path]::GetFullPath((Join-Path $projectRoot 'node_modules'))
$prefix = $projectRoot.TrimEnd('\') + '\'
if (-not $cachedModules.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or
    -not $normalModules.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Dependency path escaped project directory'
}
if (Test-Path -LiteralPath $normalModules) { Write-Output 'node_modules already exists; nothing moved'; exit 0 }
if (-not (Test-Path -LiteralPath $cachedModules)) { throw 'No cached dependencies; run npm ci instead' }
Move-Item -LiteralPath $cachedModules -Destination $normalModules
Write-Output 'Restored local dependencies to node_modules for standard npm commands'
