$ErrorActionPreference = 'Stop'
$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User') + ';' + $env:Path
try {
    & (Join-Path $PSScriptRoot 'video_loader.exe')
    if ($LASTEXITCODE -ne 0) {
        Write-Host "VideoLoader exit code: $LASTEXITCODE"
        Read-Host 'Press Enter to close'
    }
} catch {
    Write-Host $_ -ForegroundColor Red
    Read-Host 'Press Enter to close'
}
