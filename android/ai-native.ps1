[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet(
        "help", "install", "launch", "status", "dump", "projects", "conversations",
        "select-project", "select-conversation", "new", "list", "draft", "send", "steer",
        "interrupt", "retry", "disconnect", "pair", "connect-host", "smoke", "logs",
        "clear-logs", "screenshot", "ui-dump"
    )]
    [string]$Command = "status",

    [string]$Id,
    [string]$ProjectId,
    [ValidateSet("codex", "grok")]
    [string]$Provider,
    [string]$Text,
    [string]$Out
)

$ErrorActionPreference = "Stop"
$Package = "dev.agentremote.messenger"
$Receiver = "$Package/dev.agentremote.messenger.debug.NativeDebugCommandReceiver"
$Action = "$Package.DEBUG_COMMAND"
$ResultFile = "files/agent-remote-native-result.json"
$Adb = if ($env:ADB) { $env:ADB } else { "adb" }

function Invoke-Adb {
    param([string[]]$AdbArguments)
    & $Adb @AdbArguments
    if ($LASTEXITCODE -ne 0) {
        throw "adb failed with exit code $LASTEXITCODE"
    }
}

function Require-Value {
    param([string]$Name, [string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "-$Name is required for '$Command'"
    }
}

function Invoke-Native {
    param(
        [string]$NativeCommand,
        [hashtable]$Extras = @{}
    )
    $AdbArguments = @(
        "shell", "am", "broadcast", "--receiver-foreground",
        "-a", $Action,
        "-n", $Receiver,
        "--es", "command", $NativeCommand
    )
    foreach ($Entry in $Extras.GetEnumerator()) {
        if ($null -ne $Entry.Value -and "$($Entry.Value)" -ne "") {
            $AdbArguments += @("--es", "$($Entry.Key)", "$($Entry.Value)")
        }
    }

    & $Adb "shell" "run-as" $Package "rm" "-f" $ResultFile 2>$null | Out-Null
    $BroadcastOutput = & $Adb @AdbArguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "adb broadcast failed: $($BroadcastOutput -join [Environment]::NewLine)"
    }

    $JsonOutput = & $Adb "shell" "run-as" $Package "cat" $ResultFile 2>$null
    if ($LASTEXITCODE -ne 0 -or $null -eq $JsonOutput) {
        Write-Output ($BroadcastOutput -join [Environment]::NewLine)
        throw "native debug receiver did not produce a fresh JSON result"
    }

    $Json = ($JsonOutput -join [Environment]::NewLine).Trim()
    if ([string]::IsNullOrWhiteSpace($Json)) {
        Write-Output ($BroadcastOutput -join [Environment]::NewLine)
        throw "native debug receiver produced an empty result"
    }

    Write-Output $Json
    try {
        $Parsed = $Json | ConvertFrom-Json
    } catch {
        throw "native debug receiver returned invalid JSON"
    }
    if ($Parsed.ok -eq $true -and $Parsed.command -ne $NativeCommand) {
        throw "native debug result command mismatch: expected $NativeCommand, got $($Parsed.command)"
    }
    if ($Parsed.ok -ne $true) {
        throw "$($Parsed.code): $($Parsed.message)"
    }
}

function Wait-NativeReady {
    param([int]$Attempts = 25)

    for ($Attempt = 1; $Attempt -le $Attempts; $Attempt++) {
        try {
            $Result = Invoke-Native "status"
            return $Result
        } catch {
            if ($_.Exception.Message -notmatch "app_not_ready") {
                throw
            }
            if ($Attempt -lt $Attempts) {
                Start-Sleep -Milliseconds 200
            }
        }
    }
    throw "Agent Remote did not expose its native debug bridge after $Attempts attempts"
}

switch ($Command) {
    "help" {
        Write-Host "Agent Remote Android AI-native debug commands"
        Write-Host "  .\ai-native.ps1 install"
        Write-Host "  .\ai-native.ps1 launch"
        Write-Host "  .\ai-native.ps1 status|dump|projects|conversations [-ProjectId UUID]"
        Write-Host "  .\ai-native.ps1 select-project -Id UUID [-Provider codex|grok]"
        Write-Host "  .\ai-native.ps1 select-conversation -Id UUID"
        Write-Host "  .\ai-native.ps1 new | list"
        Write-Host "  .\ai-native.ps1 draft -Text '...'"
        Write-Host "  .\ai-native.ps1 send [-Text '...'] | steer [-Text '...'] | interrupt"
        Write-Host "  .\ai-native.ps1 pair -Text '<pair-url>' | connect-host -Id UUID"
        Write-Host "  .\ai-native.ps1 smoke | logs | screenshot [-Out path] | ui-dump [-Out path]"
    }
    "install" {
        & "$PSScriptRoot\gradlew.bat" ":app:installDebug"
        if ($LASTEXITCODE -ne 0) { throw "Gradle installDebug failed" }
    }
    "launch" {
        Invoke-Adb @("shell", "am", "start", "-W", "-n", "$Package/.MainActivity")
    }
    "status" { Invoke-Native "status" }
    "dump" { Invoke-Native "dump" @{ project_id = $ProjectId } }
    "projects" { Invoke-Native "projects" }
    "conversations" { Invoke-Native "conversations" @{ project_id = $ProjectId } }
    "select-project" {
        Require-Value "Id" $Id
        Invoke-Native "select_project" @{ id = $Id; provider = $Provider }
    }
    "select-conversation" {
        Require-Value "Id" $Id
        Invoke-Native "select_conversation" @{ id = $Id }
    }
    "new" { Invoke-Native "new_conversation" }
    "list" { Invoke-Native "show_conversations" }
    "draft" {
        Require-Value "Text" $Text
        Invoke-Native "set_draft" @{ text = $Text }
    }
    "send" { Invoke-Native "send" @{ text = $Text } }
    "steer" { Invoke-Native "steer" @{ text = $Text } }
    "interrupt" { Invoke-Native "interrupt" }
    "retry" { Invoke-Native "retry" }
    "disconnect" { Invoke-Native "disconnect" }
    "pair" {
        Require-Value "Text" $Text
        Invoke-Native "pair" @{ text = $Text }
    }
    "connect-host" {
        Require-Value "Id" $Id
        Invoke-Native "connect_host" @{ id = $Id }
    }
    "smoke" {
        Invoke-Adb @("shell", "am", "start", "-W", "-n", "$Package/.MainActivity")
        Wait-NativeReady
        Invoke-Native "projects"
        Invoke-Native "conversations"
    }
    "logs" {
        Invoke-Adb @("logcat", "-d", "-s", "AgentRemoteNative:I", "*:S")
    }
    "clear-logs" { Invoke-Adb @("logcat", "-c") }
    "screenshot" {
        if ([string]::IsNullOrWhiteSpace($Out)) {
            $Out = Join-Path (Get-Location) "agent-remote-screen.png"
        }
        $Remote = "/sdcard/Download/agent-remote-screen.png"
        Invoke-Adb @("shell", "screencap", "-p", $Remote)
        Invoke-Adb @("pull", $Remote, $Out)
        Write-Host "Saved $Out"
    }
    "ui-dump" {
        if ([string]::IsNullOrWhiteSpace($Out)) {
            $Out = Join-Path (Get-Location) "agent-remote-ui.xml"
        }
        $Remote = "/sdcard/Download/agent-remote-ui.xml"
        Invoke-Adb @("shell", "uiautomator", "dump", $Remote)
        Invoke-Adb @("pull", $Remote, $Out)
        Write-Host "Saved $Out"
    }
}
