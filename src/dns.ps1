param(
    [ValidateSet('Compare', 'Apply', 'Restore')][string] $Mode,
    [guid] $Id,
    [string] $Expected
)
$ErrorActionPreference = 'Stop'
$servers = @('1.1.1.1', '1.0.0.1')

function MainAdapter {
    $route = Find-NetRoute -RemoteIPAddress '1.1.1.1' | Select-Object -First 1
    Get-NetAdapter -Physical | Where-Object { $_.ifIndex -eq $route.InterfaceIndex -and $_.Status -eq 'Up' } |
        Select-Object -First 1
}

function StaticDns($adapter) {
    $guid = ([guid]$adapter.InterfaceGuid).ToString('B')
    $path = 'HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\' + $guid
    [string](Get-ItemProperty -LiteralPath $path).NameServer
}

function QueryTime($name, $server) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    Resolve-DnsName -Name $name -Server $server -Type A -DnsOnly -NoHostsFile -QuickTimeout |
        Out-Null
    $watch.Elapsed.TotalMilliseconds
}

try {
    if ($Mode -eq 'Restore') {
        $adapter = Get-NetAdapter -Physical | Where-Object { [guid]$_.InterfaceGuid -eq $Id } | Select-Object -First 1
        if (-not $adapter) { exit 1 }
        $configured = (StaticDns $adapter).Trim()
        if (-not $configured) { exit 0 }
        if (($configured -split '[,\s]+' -join ',') -ne ($servers -join ',')) { exit 1 }
        $dns = Get-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4
        Set-DnsClientServerAddress -InputObject $dns -ResetServerAddresses
        if ((StaticDns $adapter).Trim()) { exit 1 }
    } else {
        $adapter = MainAdapter
        if (-not $adapter) {
            if ($Mode -eq 'Compare') { 'unsupported'; exit 0 }
            exit 1
        }
        $dns = Get-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4
        $current = $dns.ServerAddresses | Select-Object -First 1
        $ip = Get-NetIPInterface -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4
        $automatic = $ip.Dhcp -eq 'Enabled' -and -not (StaticDns $adapter).Trim()
        if ($Mode -eq 'Compare') {
            if (-not $current) { exit 1 }
            Import-Module DnsClient
            $currentTimes = @()
            $candidateTimes = @()
            $names = @('www.microsoft.com.', 'www.cloudflare.com.', 'www.wikipedia.org.')
            for ($i = 0; $i -lt $names.Length; $i++) {
                if ($i % 2 -eq 0) {
                    $candidateTimes += QueryTime $names[$i] $servers[0]
                    $currentTimes += QueryTime $names[$i] $current
                } else {
                    $currentTimes += QueryTime $names[$i] $current
                    $candidateTimes += QueryTime $names[$i] $servers[0]
                }
            }
            @{
                id = ([guid]$adapter.InterfaceGuid).ToString('D')
                server = $current
                current = @($currentTimes | Sort-Object)[1]
                cloudflare = @($candidateTimes | Sort-Object)[1]
                automatic = $automatic -and $current -ne $servers[0]
            } | ConvertTo-Json -Compress
            exit 0
        }
        if ([guid]$adapter.InterfaceGuid -ne $Id -or -not $automatic -or $current -ne $Expected) { exit 1 }
        Set-DnsClientServerAddress -InputObject $dns -ServerAddresses $servers -Validate
        $updated = Get-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4
        if (($updated.ServerAddresses -join ',') -ne ($servers -join ',')) { exit 1 }
    }
    Clear-DnsClientCache -ErrorAction SilentlyContinue
    exit 0
} catch {
    exit 1
}
