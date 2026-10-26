param([ValidateSet('All', 'Source', 'Resolution', 'Style')][string]$Schema = 'All')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$flatc = if ($env:FLATC) { $env:FLATC } else { 'flatc' }
$version = & $flatc --version
if ($LASTEXITCODE -ne 0 -or $version -ne 'flatc version 25.2.10') {
    throw 'Cache bindings require flatc version 25.2.10'
}
if ($Schema -in @('All', 'Source')) {
    & $flatc --rust --gen-all --gen-object-api -o "$root\crates\cache_flatbuffers\src" "$root\flatbuffers\cache.fbs"
    if ($LASTEXITCODE -ne 0) { throw 'flatc source generation failed' }
}
if ($Schema -in @('All', 'Resolution')) {
    & $flatc --rust --gen-all --gen-object-api -o "$root\crates\cache_flatbuffers\src" "$root\flatbuffers\resolution.fbs"
    if ($LASTEXITCODE -ne 0) { throw 'flatc resolution generation failed' }
}
if ($Schema -in @('All', 'Style')) {
    & $flatc --rust --gen-all --gen-object-api -o "$root\crates\cache_flatbuffers\src" "$root\flatbuffers\style.fbs"
    if ($LASTEXITCODE -ne 0) { throw 'flatc style generation failed' }
}
