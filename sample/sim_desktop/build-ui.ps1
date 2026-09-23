$ErrorActionPreference = 'Stop'
$env:NO_COLOR = 'true'
# trunk serveのdistとは別に生成し、ブラウザ開発中の出力と競合させない。
$desktopRoot = $PSScriptRoot
$toolchainBin = Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin'
if (Test-Path (Join-Path $toolchainBin 'cargo.exe')) {
    $env:PATH = "$toolchainBin;$env:PATH"
}
Push-Location (Join-Path $desktopRoot '../sim_frontend')
try {
    & trunk build --release --dist (Join-Path $desktopRoot 'out/frontend')
    if ($LASTEXITCODE -ne 0) { throw 'デスクトップ用UIのビルドに失敗しました。' }
} finally {
    Pop-Location
}
