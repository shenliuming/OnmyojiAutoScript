# MuMu full-channel login preflight. Read-only by default: instance discovery,
# package states, ADB and OAS reachability.

param(
    [string]$EnvFile = (Join-Path $PSScriptRoot "agent.env"),
    # Validate deploy/windows/agent.env.example documents every required MuMu key.
    [switch]$ValidateEnvExample
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$failures = New-Object System.Collections.Generic.List[string]

function Add-Failure([string]$Message) {
    $failures.Add($Message)
    Write-Host "[FAIL] $Message" -ForegroundColor Red
}

function Add-Success([string]$Message) {
    Write-Host "[ OK ] $Message" -ForegroundColor Green
}

function Import-EnvFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Agent env file not found: $Path"
    }

    Get-Content -LiteralPath $Path | ForEach-Object {
        $line = $_.Trim()
        if ($line.Length -eq 0 -or $line.StartsWith("#")) {
            return
        }

        if ($line -notmatch '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
            throw "Invalid env line: $line"
        }

        $name = $Matches[1]
        $value = $Matches[2].Trim()
        if (
            ($value.StartsWith('"') -and $value.EndsWith('"')) -or
            ($value.StartsWith("'") -and $value.EndsWith("'"))
        ) {
            $value = $value.Substring(1, $value.Length - 2)
        }
        [Environment]::SetEnvironmentVariable($name, $value, "Process")
    }
}

$requiredExampleKeys = @{
    "FOSTER_MUMU_CLI_PATH"                  = "mumu-cli.exe"
    "FOSTER_MUMU_APP_MARKET_PACKAGE"        = "com.mumu.store"
    "FOSTER_MUMU_APP_MARKET_ACTIVITY"       = "com.mumu.store/.MainActivity"
    "FOSTER_FULL_CHANNEL_PACKAGE"           = "com.netease.onmyoji.wyzymnqsd_cps"
    "FOSTER_NORMAL_PACKAGE"                 = "com.netease.onmyoji"
    "FOSTER_RESOLUTION_WIDTH"               = "1280"
    "FOSTER_RESOLUTION_HEIGHT"              = "720"
    "FOSTER_MUMU_COMMAND_TIMEOUT_SECONDS"   = $null
    "FOSTER_MUMU_MARKET_TIMEOUT_SECONDS"    = $null
    "FOSTER_MUMU_MARKET_POLL_SECONDS"       = $null
    "FOSTER_MUMU_LAUNCH_WAIT_SECONDS"       = $null
    "FOSTER_MUMU_LAUNCH_SETTLE_SECONDS"     = $null
}

if ($ValidateEnvExample) {
    $examplePath = Join-Path $PSScriptRoot "agent.env.example"
    if (-not (Test-Path -LiteralPath $examplePath)) {
        Add-Failure "agent.env.example not found: $examplePath"
        exit 1
    }

    $exampleLines = Get-Content -LiteralPath $examplePath
    foreach ($key in $requiredExampleKeys.Keys) {
        $expectedValue = $requiredExampleKeys[$key]
        $line = $exampleLines | Where-Object { $_ -match "^$key=" } | Select-Object -First 1
        if (-not $line) {
            Add-Failure "agent.env.example is missing required key: $key"
            continue
        }
        $value = ($line -split "=", 2)[1].Trim()
        if ($expectedValue -and $key -eq "FOSTER_MUMU_CLI_PATH") {
            if (-not $value.EndsWith("mumu-cli.exe")) {
                Add-Failure "agent.env.example $key must point at mumu-cli.exe but is '$value'"
            }
        }
        elseif ($expectedValue -and $value -ne $expectedValue) {
            Add-Failure "agent.env.example $key must be '$expectedValue' but is '$value'"
        }
    }

    if ($failures.Count -gt 0) {
        Write-Host ""
        Write-Host "Env example validation failed: $($failures.Count) issue(s)." -ForegroundColor Red
        exit 1
    }

    Write-Host ""
    Write-Host "agent.env.example documents the MuMu full-channel configuration." -ForegroundColor Green
    exit 0
}

# ---------------------------------------------------------------------------
# Read-only preflight below. Nothing here uninstalls, installs or launches.
# ---------------------------------------------------------------------------

Import-EnvFile $EnvFile

$mumuCliPath = [Environment]::GetEnvironmentVariable("FOSTER_MUMU_CLI_PATH", "Process")
if ([string]::IsNullOrWhiteSpace($mumuCliPath)) {
    $mumuCliPath = "C:\Program Files\Netease\MuMu\nx_main\mumu-cli.exe"
}
$fullChannelPackage = [Environment]::GetEnvironmentVariable("FOSTER_FULL_CHANNEL_PACKAGE", "Process")
if ([string]::IsNullOrWhiteSpace($fullChannelPackage)) {
    $fullChannelPackage = "com.netease.onmyoji.wyzymnqsd_cps"
}
$normalPackage = [Environment]::GetEnvironmentVariable("FOSTER_NORMAL_PACKAGE", "Process")
if ([string]::IsNullOrWhiteSpace($normalPackage)) {
    $normalPackage = "com.netease.onmyoji"
}

$adbPath = [Environment]::GetEnvironmentVariable("FOSTER_ADB_PATH", "Process")
if ([string]::IsNullOrWhiteSpace($adbPath)) {
    $adbPath = "adb"
}

$oasBaseUrl = [Environment]::GetEnvironmentVariable("FOSTER_OAS_BASE_URL", "Process")
if ([string]::IsNullOrWhiteSpace($oasBaseUrl)) {
    $oasBaseUrl = "http://127.0.0.1:22270"
}
$oasBaseUrl = $oasBaseUrl.TrimEnd("/")

if (-not (Test-Path -LiteralPath $mumuCliPath)) {
    Add-Failure "MuMu CLI not found: $mumuCliPath (set FOSTER_MUMU_CLI_PATH)"
}
else {
    Add-Success "MuMu CLI found: $mumuCliPath"
}

$adbCommand = Get-Command $adbPath -ErrorAction SilentlyContinue
if (-not $adbCommand -and -not (Test-Path -LiteralPath $adbPath)) {
    Add-Failure "ADB executable not found: $adbPath"
}
else {
    Add-Success "ADB executable found: $adbPath"
}

$instances = @()
if (Test-Path -LiteralPath $mumuCliPath) {
    try {
        $infoRaw = (& $mumuCliPath info --vmindex all 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0) {
            Add-Failure "MuMu CLI info failed with exit code $LASTEXITCODE"
        }
        else {
            $info = $infoRaw | ConvertFrom-Json
            $data = $info
            if ($info.PSObject.Properties["data"]) { $data = $info.data }
            elseif ($info.PSObject.Properties["players"]) { $data = $info.players }

            if ($data -is [System.Management.Automation.PSCustomObject]) {
                $instances = @($data.PSObject.Properties | ForEach-Object { $_.Value })
            }
            else {
                $instances = @($data)
            }

            if ($instances.Count -eq 0) {
                Add-Failure "MuMu CLI reported no instances"
            }
            else {
                Add-Success "MuMu instances discovered: $($instances.Count)"
                foreach ($instance in $instances) {
                    $index = $instance.index
                    $name = $instance.name
                    $androidStarted = [bool]$instance.is_android_started
                    $processStarted = [bool]$instance.is_process_started
                    $playerState = [string]$instance.player_state
                    $adbEndpoint = ""
                    if ($instance.adb_host_ip -and $instance.adb_port) {
                        $adbEndpoint = "$($instance.adb_host_ip):$($instance.adb_port)"
                    }
                    Write-Host (
                        "  instance {0}: {1} android_started={2} process_started={3} state={4} adb={5}" -f
                        $index, $name, $androidStarted, $processStarted, $playerState, ($(if ($adbEndpoint) { $adbEndpoint } else { "n/a" }))
                    )
                }
            }
        }
    }
    catch {
        Add-Failure "MuMu CLI info could not be parsed: $($_.Exception.Message)"
    }
}

foreach ($instance in $instances) {
    $androidStarted = [bool]$instance.is_android_started
    if (-not $androidStarted) { continue }
    if (-not $instance.adb_host_ip -or -not $instance.adb_port) { continue }
    $serial = "$($instance.adb_host_ip):$($instance.adb_port)"
    if (-not $adbCommand -and -not (Test-Path -LiteralPath $adbPath)) { continue }

    try {
        # Network serials need an explicit (idempotent) connect first.
        & $adbPath connect $serial 2>&1 | Out-Null
        $packagesRaw = (& $adbPath -s $serial shell pm list packages 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0) {
            Add-Failure "$serial package query failed"
            continue
        }
        $hasNormal = $packagesRaw -match "package:$normalPackage"
        $hasFullChannel = $packagesRaw -match "package:$fullChannelPackage"
        Write-Host (
            "  instance {0} ({1}): normal_package={2} full_channel_package={3}" -f
            $instance.index, $serial, $(if ($hasNormal) { "present" } else { "absent" }), $(if ($hasFullChannel) { "present" } else { "absent" })
        )
        if ($hasNormal) {
            Write-Host "    note: $normalPackage is removed automatically on every instance during login preparation (destructive: local data of that package is deleted)." -ForegroundColor Yellow
        }
    }
    catch {
        Add-Failure "$serial package query failed: $($_.Exception.Message)"
    }
}

try {
    $null = Invoke-RestMethod -Method Get -Uri "$oasBaseUrl/openapi.json" -TimeoutSec 5
    Add-Success "OAS FastAPI reachable: $oasBaseUrl"
}
catch {
    Add-Failure "Cannot reach OAS FastAPI: $($_.Exception.Message)"
}

if ($failures.Count -gt 0) {
    Write-Host ""
    Write-Host "MuMu full-channel preflight failed: $($failures.Count) issue(s)." -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "MuMu full-channel preflight passed (read-only)." -ForegroundColor Green
exit 0
