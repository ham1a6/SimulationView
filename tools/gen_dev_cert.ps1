# 開発用の自己署名証明書(HTTPS/WSS用)を certs/ に生成する。
# trunk serve(8081)とsim_server(9001)の両方が同じ証明書を使う。
#
#   pwsh tools/gen_dev_cert.ps1                 # localhost/127.0.0.1/このPCのホスト名・LAN IPを含める
#   pwsh tools/gen_dev_cert.ps1 -ExtraHost foo  # 追加のDNS名/IPを含める(複数可)
#
# 出力(certs/はgit管理外): certs/dev-cert.pem(証明書) と certs/dev-key.pem(秘密鍵)。
# 接続元のIPが変わったら(DHCP等)再実行すること。ブラウザ側の信頼設定はREADME.mdの「HTTPS」節。
param(
    [string[]]$ExtraHost = @(),
    [int]$Days = 825
)

$ErrorActionPreference = "Stop"

$openssl = (Get-Command openssl -ErrorAction SilentlyContinue).Source
if (-not $openssl) {
    $openssl = "C:\Program Files\Git\usr\bin\openssl.exe"
}
if (-not (Test-Path $openssl)) {
    throw "openssl.exe が見つかりません。Git for Windows同梱のもの、または別途インストールしたものをPATHに通してください。"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$certDir = Join-Path $repoRoot "certs"
New-Item -ItemType Directory -Force $certDir | Out-Null

# SAN(subjectAltName)。ブラウザはCN(件名)ではなくSANでホスト名を照合する。
$dns = @("localhost", [System.Net.Dns]::GetHostName())
$ips = @("127.0.0.1", "::1")
$lanIps = Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
    Where-Object { $_.IPAddress -notlike "169.254.*" -and $_.IPAddress -ne "127.0.0.1" } |
    ForEach-Object { $_.IPAddress }
$ips += $lanIps
foreach ($h in $ExtraHost) {
    if ($h -match '^[0-9.]+$' -or $h -match ':') { $ips += $h } else { $dns += $h }
}
$san = (($dns | Select-Object -Unique | ForEach-Object { "DNS:$_" }) +
        ($ips | Select-Object -Unique | ForEach-Object { "IP:$_" })) -join ","

$cert = Join-Path $certDir "dev-cert.pem"
$key = Join-Path $certDir "dev-key.pem"

# 自己署名のためCA:TRUEにして、そのまま「信頼されたルート証明機関」へ登録できるようにする。
& $openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days $Days `
    -keyout $key -out $cert `
    -subj "/CN=Sim3dView Dev" `
    -addext "subjectAltName=$san" `
    -addext "basicConstraints=critical,CA:TRUE" `
    -addext "keyUsage=critical,digitalSignature,keyEncipherment,keyCertSign" `
    -addext "extendedKeyUsage=serverAuth"
if ($LASTEXITCODE -ne 0) { throw "openssl req が失敗しました" }

Write-Host "生成しました:"
Write-Host "  証明書: $cert"
Write-Host "  秘密鍵: $key"
Write-Host "  SAN   : $san"
