# One-time migration of administrator-created tasks. Normal app use needs no elevation.
[CmdletBinding(SupportsShouldProcess)]
param(
    [string]$Executable,
    [string]$UserSid = ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value),
    [string]$BackupDirectory = (Join-Path $env:LOCALAPPDATA ('JobScraper\startup-repair-' + (Get-Date -Format 'yyyyMMdd-HHmmss')))
)
$ErrorActionPreference = 'Stop'
if (-not $Executable) {
    $Executable = if (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'jobscraper.exe')) {
        Join-Path $PSScriptRoot 'jobscraper.exe'
    } else {
        Join-Path $env:LOCALAPPDATA 'Programs\JobScraper\jobscraper.exe'
    }
}
$sid = [Security.Principal.SecurityIdentifier]::new($UserSid)
$executablePath = (Resolve-Path -LiteralPath $Executable).ProviderPath
if ([IO.Path]::GetFileName($executablePath) -ine 'jobscraper.exe') { throw 'Expected the installed jobscraper.exe.' }
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $WhatIfPreference -and -not ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this repair once from an administrator PowerShell. Then run JobScraper normally.'
}
$service = New-Object -ComObject 'Schedule.Service'
$service.Connect()
$folder = $null
try { $folder = $service.GetFolder('\JobScraper') }
catch { if ($_.Exception.HResult -notin @(-2147024894, -2147024893)) { throw } }
$tasks = @()
if ($folder) {
    foreach ($task in $folder.GetTasks(1)) {
        if ($task.Name -notmatch '^JobScraper-(Startup|Sync|Reminder-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$') { continue }
        [xml]$xml = $task.Xml
        $ns = [Xml.XmlNamespaceManager]::new($xml.NameTable)
        $ns.AddNamespace('t', $xml.DocumentElement.NamespaceURI)
        $commands = @($xml.SelectNodes('/t:Task/t:Actions/t:Exec/t:Command', $ns))
        if ($commands.Count -ne 1 -or [IO.Path]::GetFullPath($commands[0].InnerText) -ine $executablePath) { continue }
        $principal = $xml.SelectSingleNode('/t:Task/t:Principals/t:Principal/t:UserId', $ns).InnerText
        if ($principal -notlike 'S-1-*') { $principal = ([Security.Principal.NTAccount]::new($principal)).Translate([Security.Principal.SecurityIdentifier]).Value }
        if ($principal -ne $sid.Value) { continue }
        $tasks += [pscustomobject]@{ Name = $task.Name; Xml = $task.Xml; Security = $task.GetSecurityDescriptor(7) }
    }
}
$layers = @()
foreach ($key in @("Registry::HKEY_USERS\$UserSid\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers", 'Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers')) {
    if (Test-Path -LiteralPath $key) {
        $value = (Get-Item -LiteralPath $key).GetValue($executablePath, $null)
        if ($null -ne $value) { $layers += [pscustomobject]@{ Key = $key; Value = [string]$value } }
    }
}
if (-not $PSCmdlet.ShouldProcess($executablePath, "Back up and repair $($tasks.Count) JobScraper tasks and this EXE's administrator compatibility flag")) { return }
New-Item -ItemType Directory -Path $BackupDirectory -ErrorAction Stop | Out-Null
@{ Executable = $executablePath; UserSid = $UserSid; Tasks = $tasks; Compatibility = $layers } |
    ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $BackupDirectory 'before.json') -Encoding UTF8

function Set-TaskValue([xml]$Document, [Xml.XmlNamespaceManager]$Namespaces, [string]$ParentPath, [string]$Name, [string]$Value) {
    $parent = $Document.SelectSingleNode($ParentPath, $Namespaces)
    if (-not $parent) { throw "Missing task element: $ParentPath" }
    $node = $parent.SelectSingleNode("t:$Name", $Namespaces)
    if (-not $node) { $node = $parent.AppendChild($Document.CreateElement($Name, $Document.DocumentElement.NamespaceURI)) }
    $node.InnerText = $Value
}
foreach ($entry in $tasks) {
    [xml]$xml = $entry.Xml
    $ns = [Xml.XmlNamespaceManager]::new($xml.NameTable)
    $ns.AddNamespace('t', $xml.DocumentElement.NamespaceURI)
    Set-TaskValue $xml $ns '/t:Task/t:Principals/t:Principal' 'UserId' $UserSid
    Set-TaskValue $xml $ns '/t:Task/t:Principals/t:Principal' 'LogonType' 'InteractiveToken'
    Set-TaskValue $xml $ns '/t:Task/t:Principals/t:Principal' 'RunLevel' 'LeastPrivilege'
    foreach ($trigger in $xml.SelectNodes('/t:Task/t:Triggers/t:LogonTrigger/t:UserId', $ns)) { $trigger.InnerText = $UserSid }
    Set-TaskValue $xml $ns '/t:Task/t:Settings' 'DisallowStartIfOnBatteries' 'false'
    Set-TaskValue $xml $ns '/t:Task/t:Settings' 'StopIfGoingOnBatteries' 'false'
    if ($entry.Name -eq 'JobScraper-Startup') {
        Set-TaskValue $xml $ns '/t:Task/t:Settings' 'ExecutionTimeLimit' 'PT0S'
        Set-TaskValue $xml $ns '/t:Task/t:Actions/t:Exec' 'Arguments' '--hidden'
    }
    # Keep disabled tasks disabled, and preserve sync/reminder schedules and arguments.
    $security = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;$UserSid)"
    $registered = $folder.RegisterTask($entry.Name, $xml.OuterXml, 6, $UserSid, $null, 3, $security)
    $registered.SetSecurityDescriptor($security, 0)
}
foreach ($layer in $layers) {
    $flags = @($layer.Value -split '\s+' | Where-Object { $_ -and $_ -ine 'RUNASADMIN' })
    if ($flags -notcontains 'RUNASADMIN' -and $layer.Value -match '(?i)(^|\s)RUNASADMIN(\s|$)') {
        if (@($flags | Where-Object { $_ -ne '~' }).Count -eq 0) {
            Remove-ItemProperty -LiteralPath $layer.Key -Name $executablePath
        } else {
            Set-ItemProperty -LiteralPath $layer.Key -Name $executablePath -Value ($flags -join ' ')
        }
    }
}
Write-Output "Repaired $($tasks.Count) tasks. Backup: $BackupDirectory\before.json"
