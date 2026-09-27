param(
    [string]$EnvFile = (Join-Path $PSScriptRoot "agent.env")
)

$ErrorActionPreference = "Stop"
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

function Get-RequiredEnv([string]$Name) {
    $value = [Environment]::GetEnvironmentVariable($Name, "Process")
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "Required environment variable is missing: $Name"
    }
    return $value
}

Import-EnvFile $EnvFile

try {
    $serverWsUrl = Get-RequiredEnv "FOSTER_SERVER_WS_URL"
    $null = Get-RequiredEnv "FOSTER_AGENT_TOKEN"
    $null = Get-RequiredEnv "FOSTER_AGENT_ID"
    $null = Get-RequiredEnv "FOSTER_HOST_ID"
    $emulatorsJson = Get-RequiredEnv "FOSTER_EMULATORS_JSON"
}
catch {
    Add-Failure $_.Exception.Message
    exit 1
}

$oasBaseUrl = [Environment]::GetEnvironmentVariable("FOSTER_OAS_BASE_URL", "Process")
if ([string]::IsNullOrWhiteSpace($oasBaseUrl)) {
    $oasBaseUrl = "http://127.0.0.1:22270"
}
$oasBaseUrl = $oasBaseUrl.TrimEnd("/")

$adbPath = [Environment]::GetEnvironmentVariable("FOSTER_ADB_PATH", "Process")
if ([string]::IsNullOrWhiteSpace($adbPath)) {
    $adbPath = "adb"
}

try {
    $wsUri = [Uri]$serverWsUrl
    if ($wsUri.Scheme -eq "ws") {
        $httpScheme = "http"
    }
    elseif ($wsUri.Scheme -eq "wss") {
        $httpScheme = "https"
    }
    else {
        throw "FOSTER_SERVER_WS_URL must use ws:// or wss://"
    }
    $serverBaseUrl = "{0}://{1}" -f $httpScheme, $wsUri.Authority
}
catch {
    Add-Failure "Invalid FOSTER_SERVER_WS_URL: $($_.Exception.Message)"
    $serverBaseUrl = $null
}

if ($serverBaseUrl) {
    try {
        $ready = Invoke-RestMethod -Method Get -Uri "$serverBaseUrl/readyz" -TimeoutSec 5
        if ($ready.status -eq "ready") {
            Add-Success "Foster Server ready: $serverBaseUrl"
        }
        else {
            Add-Failure "Foster Server returned unexpected readiness response"
        }
    }
    catch {
        Add-Failure "Cannot reach Foster Server /readyz: $($_.Exception.Message)"
    }
}

try {
    $null = Invoke-RestMethod -Method Get -Uri "$oasBaseUrl/openapi.json" -TimeoutSec 5
    Add-Success "OAS FastAPI reachable: $oasBaseUrl"
}
catch {
    Add-Failure "Cannot reach OAS FastAPI: $($_.Exception.Message)"
}

try {
    $emulators = @($emulatorsJson | ConvertFrom-Json)
}
catch {
    Add-Failure "FOSTER_EMULATORS_JSON is invalid JSON: $($_.Exception.Message)"
    $emulators = @()
}

if ($emulators.Count -eq 0) {
    Add-Failure "FOSTER_EMULATORS_JSON contains no emulator"
}

$adbCommand = Get-Command $adbPath -ErrorAction SilentlyContinue
if (-not $adbCommand -and -not (Test-Path -LiteralPath $adbPath)) {
    Add-Failure "ADB executable not found: $adbPath"
}
else {
    Add-Success "ADB executable found: $adbPath"

    foreach ($emulator in $emulators) {
        $code = [string]$emulator.emulatorCode
        $serial = [string]$emulator.adbSerial
        $oasConfig = [string]$emulator.oasConfigName

        if ([string]::IsNullOrWhiteSpace($code) -or
            [string]::IsNullOrWhiteSpace($serial) -or
            [string]::IsNullOrWhiteSpace($oasConfig)) {
            Add-Failure "Emulator entry requires emulatorCode, adbSerial and oasConfigName"
            continue
        }

        try {
            # Only TCP endpoints need adb connect. LDPlayer serials such as
            # emulator-5554 are already registered and must be addressed directly.
            if ($serial.Contains(":")) {
                & $adbPath connect $serial 2>&1 | Out-Null
            }
            $state = (& $adbPath -s $serial get-state 2>&1 | Out-String).Trim()
            if ($LASTEXITCODE -eq 0 -and $state -eq "device") {
                Add-Success "$code ADB online ($serial), OAS config=$oasConfig"
            }
            else {
                Add-Failure "$code ADB is not ready ($serial): $state"
            }
        }
        catch {
            Add-Failure "$code ADB check failed ($serial): $($_.Exception.Message)"
        }
    }
}

$ldConsolePaths = @(
    $emulators |
        ForEach-Object { [string]$_.startProgram } |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
        Select-Object -Unique
)

if ($ldConsolePaths.Count -eq 0) {
    Add-Failure "No LDPlayer startProgram configured in FOSTER_EMULATORS_JSON"
}
else {
    foreach ($ldConsolePath in $ldConsolePaths) {
        if (-not (Test-Path -LiteralPath $ldConsolePath)) {
            Add-Failure "LDPlayer ldconsole.exe not found: $ldConsolePath"
            continue
        }

        Add-Success "LDPlayer CLI found: $ldConsolePath"
        try {
            $listRaw = (& $ldConsolePath list2 2>&1 | Out-String).Trim()
            if ($LASTEXITCODE -ne 0) {
                Add-Failure "LDPlayer list2 failed with exit code $LASTEXITCODE"
            }
            elseif ([string]::IsNullOrWhiteSpace($listRaw)) {
                Add-Failure "LDPlayer list2 returned no instances"
            }
            else {
                Add-Success "LDPlayer list2 responds"
                $listRaw -split "\r?\n" | ForEach-Object {
                    if (-not [string]::IsNullOrWhiteSpace($_)) {
                        Write-Host "  $_"
                    }
                }
            }
        }
        catch {
            Add-Failure "LDPlayer list2 failed: $($_.Exception.Message)"
        }
    }
}

if ($failures.Count -gt 0) {
    Write-Host ""
    Write-Host "Smoke check failed: $($failures.Count) issue(s)." -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Foster host preflight passed." -ForegroundColor Green
exit 0
