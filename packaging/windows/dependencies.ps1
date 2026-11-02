$ErrorActionPreference = 'Stop'
function Refresh-Path {
    $env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User') + ';' + $env:Path
}
function Works([string] $Program, [string] $Argument) {
    if (-not (Get-Command $Program -ErrorAction SilentlyContinue)) { return $false }
    try {
        & $Program $Argument *> $null
        return $LASTEXITCODE -eq 0
    } catch { return $false }
}
try {
    Refresh-Path
    Write-Host 'VideoLoader - Windows dependencies' -ForegroundColor Cyan
    $packages = @()
    if (-not (Works 'yt-dlp' '--version')) { $packages += 'yt-dlp.yt-dlp' }
    if (-not (Works 'ffmpeg' '-version') -or -not (Works 'ffprobe' '-version')) { $packages += 'yt-dlp.FFmpeg' }
    if (-not (Works 'deno' '--version')) { $packages += 'DenoLand.Deno' }
    if ($packages.Count -eq 0) {
        Write-Host 'yt-dlp, FFmpeg, FFprobe and Deno are available.' -ForegroundColor Green
    } else {
        if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
            throw 'WinGet not found. Install or update Microsoft App Installer from Microsoft Store, then run this check again.'
        }
        Write-Host 'Proposed commands:'
        foreach ($id in $packages) { Write-Host "winget install --exact --id $id --source winget" }
        Write-Host 'Internet is required. Review agreements and any Windows permission prompts.'
        Write-Host 'Dependencies are separate; uninstalling VideoLoader will NOT remove them.'
        $answer = Read-Host 'Type YES to continue, or press Enter to cancel'
        if ($answer -ceq 'YES') {
            foreach ($id in $packages) {
                & winget install --exact --id $id --source winget
                if ($LASTEXITCODE -ne 0) { Write-Host "WinGet returned $LASTEXITCODE for $id. Review output above." -ForegroundColor Yellow }
            }
            Refresh-Path
            foreach ($program in @('yt-dlp','ffmpeg','ffprobe','deno')) {
                $argument = if ($program -in @('ffmpeg','ffprobe')) { '-version' } else { '--version' }
                $ok = Works $program $argument
                Write-Host ($program + ': ' + $(if ($ok) { 'OK' } else { 'NOT AVAILABLE' }))
            }
            Write-Host 'Close and reopen VideoLoader using its Start menu shortcut to refresh PATH.'
        } else { Write-Host 'Installation cancelled.' }
    }
    Write-Host 'Verify actual YouTube support by analyzing a video in VideoLoader.'
} catch { Write-Host $_ -ForegroundColor Red }
Read-Host 'Press Enter to close'
