param(
    [string]$AgentDir = $PSScriptRoot,
    [string]$TaskName = "FosterAgent",
    [string]$RunAsUser = ([System.Security.Principal.WindowsIdentity]::GetCurrent().Name)
)

$ErrorActionPreference = "Stop"

$AgentDir = (Resolve-Path -LiteralPath $AgentDir).Path
$StartScript = Join-Path $AgentDir "Start-FosterAgent.ps1"

if (-not (Test-Path -LiteralPath $StartScript)) {
    throw "Start-FosterAgent.ps1 not found in: $AgentDir"
}

if ([string]::IsNullOrWhiteSpace($RunAsUser)) {
    throw "RunAsUser is required"
}

$argument = '-NoProfile -ExecutionPolicy Bypass -File "{0}"' -f $StartScript

$action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument $argument -WorkingDirectory $AgentDir
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $RunAsUser
$principal = New-ScheduledTaskPrincipal -UserId $RunAsUser -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -RestartCount 10 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null

Write-Host "Scheduled task '$TaskName' installed for interactive user: $RunAsUser"
Write-Host "The task starts when that user logs on."
Write-Host "Start it now with: Start-ScheduledTask -TaskName '$TaskName'"
