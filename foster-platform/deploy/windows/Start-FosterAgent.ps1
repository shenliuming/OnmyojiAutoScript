param(
    [string]$EnvFile = (Join-Path $PSScriptRoot "agent.env"),
    [string]$AgentExe = (Join-Path $PSScriptRoot "foster-agent.exe")
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $EnvFile)) {
    throw "Agent env file not found: $EnvFile"
}

if (-not (Test-Path -LiteralPath $AgentExe)) {
    throw "foster-agent.exe not found: $AgentExe"
}

Get-Content -LiteralPath $EnvFile | ForEach-Object {
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

$required = @(
    "FOSTER_SERVER_WS_URL",
    "FOSTER_AGENT_TOKEN",
    "FOSTER_AGENT_ID",
    "FOSTER_HOST_ID",
    "FOSTER_EMULATORS_JSON"
)

foreach ($name in $required) {
    $value = [Environment]::GetEnvironmentVariable($name, "Process")
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "Required environment variable is missing: $name"
    }
}

try {
    $null = $env:FOSTER_EMULATORS_JSON | ConvertFrom-Json
}
catch {
    throw "FOSTER_EMULATORS_JSON is not valid JSON: $($_.Exception.Message)"
}

Push-Location $PSScriptRoot
try {
    & $AgentExe
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
