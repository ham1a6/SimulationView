$ErrorActionPreference = 'Stop'
# 既存の直接呼び出しは残し、実処理をOS共通スクリプトへ委譲する。
& node (Join-Path $PSScriptRoot 'build-ui.cjs')
if ($LASTEXITCODE -ne 0) { throw 'デスクトップ用UIのビルドに失敗しました。' }
