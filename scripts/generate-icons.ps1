[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$assetDirectory = Join-Path $PSScriptRoot '../assets'
$source = Join-Path $assetDirectory 'logo.svg'
$png = Join-Path $assetDirectory 'icon.png'
$ico = Join-Path $assetDirectory 'icon.ico'
# The SVG is the editable source. Generated icons are committed so normal
# application builds do not require ImageMagick.
& magick -background none -density 384 $source -resize 512x512 $png
if ($LASTEXITCODE -ne 0) { throw 'PNG icon generation failed.' }
& magick $png -define icon:auto-resize=256,128,64,48,32,24,16 $ico
if ($LASTEXITCODE -ne 0) { throw 'Windows icon generation failed.' }
Write-Output 'Generated assets/icon.png and assets/icon.ico.'
