param(
    [Parameter(Mandatory)]
    [string]$PackagePath,

    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+\.0$')]
    [string]$ExpectedVersion
)

$ErrorActionPreference = 'Stop'
$assetPrefix = 'meta-harness-desktop-windows-x64'
$packageName = 'pan2013e.Meta-Harness'
$publisher = 'CN=EB6DF389-F6E4-4979-B86E-56C12C4F0020'

function Get-StorePackageIdentity([string]$Path) {
    $zip = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $bundle = $zip.GetEntry('AppxMetadata/AppxBundleManifest.xml')
        $manifest = if ($bundle) { $bundle } else { $zip.GetEntry('AppxManifest.xml') }
        if (!$manifest -or !$zip.GetEntry('AppxSignature.p7x')) { return }
        $stream = $manifest.Open()
        try {
            $xml = [xml]::new()
            $xml.Load($stream)
            if ($bundle) {
                $x64 = $xml.Bundle.Packages.Package | Where-Object { $_.Type -ceq 'application' -and $_.Architecture -ceq 'x64' }
                if ($x64) {
                    return [pscustomobject]@{ Identity = $xml.Bundle.Identity; Extension = '.appxbundle' }
                }
            } elseif ($xml.Package.Identity.ProcessorArchitecture -ceq 'x64') {
                return [pscustomobject]@{ Identity = $xml.Package.Identity; Extension = '.appx' }
            }
        } finally {
            $stream.Dispose()
        }
    } finally {
        $zip.Dispose()
    }
}

$package = Get-Item -LiteralPath $PackagePath
$manifest = Get-StorePackageIdentity $package.FullName
$identity = $manifest.Identity
if ($identity.Name -cne $packageName -or $identity.Publisher -cne $publisher -or $identity.Version -cne $ExpectedVersion) {
    throw 'Expected a signed x64 Meta-Harness package matching the reserved Store identity and catalog version.'
}
$assetName = $assetPrefix + $manifest.Extension
$asset = Join-Path $package.DirectoryName $assetName
if ($asset -ne $package.FullName) { Copy-Item -LiteralPath $package.FullName -Destination $asset }
if ((Get-AuthenticodeSignature -LiteralPath $asset).Status -ne 'Valid') {
    throw 'The Store package signature could not be verified.'
}

$tag = 'v' + $ExpectedVersion.Substring(0, $ExpectedVersion.Length - 2)
$release = gh api "repos/$env:GITHUB_REPOSITORY/releases/tags/$tag" 2>$null | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    Write-Output "The GitHub release $tag is unavailable. A later run will retry."
    return
}
if ($release.draft -or $release.prerelease) {
    Write-Output "Waiting for a public stable GitHub release for $tag."
    return
}
if ($release.assets.name | Where-Object { $_ -in "$assetPrefix.appx", "$assetPrefix.appxbundle" }) {
    Write-Output "The Store package is already attached to $tag."
    return
}

gh release upload $tag $asset --repo $env:GITHUB_REPOSITORY
if ($LASTEXITCODE -ne 0) { throw 'Could not attach the signed package to the GitHub release.' }
