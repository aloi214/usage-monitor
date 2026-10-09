# Inspect build outputs without launching the app or installer.
$ErrorActionPreference = 'Stop'
$config = Get-Content -Raw src-tauri/tauri.conf.json | ConvertFrom-Json
$package = Get-Content -Raw package.json | ConvertFrom-Json
$exe = Get-Item "src-tauri/target/release/$($package.name).exe"
if ($exe.VersionInfo.ProductName -cne $config.productName) {
  throw "Executable ProductName does not match $($config.productName)."
}
if ($exe.VersionInfo.ProductVersion -cne $config.version) {
  throw "Executable ProductVersion does not match $($config.version)."
}
$installers = @(Get-ChildItem src-tauri/target/release/bundle/nsis/*-setup.exe -File)
if ($installers.Count -ne 1) { throw 'Expected exactly one NSIS installer.' }
$installer = $installers[0]
$expectedName = "$($config.productName)_$($config.version)_x64-setup.exe"
if ($installer.Name -cne $expectedName) { throw "Expected installer $expectedName, found $($installer.Name)." }
if ($installer.VersionInfo.ProductName -cne $config.productName) {
  throw "Installer ProductName does not match $($config.productName)."
}
Write-Output "Verified $($exe.Name): $($exe.VersionInfo.ProductName) $($exe.VersionInfo.ProductVersion)"
Write-Output "Verified installer: $($installer.Name)"
