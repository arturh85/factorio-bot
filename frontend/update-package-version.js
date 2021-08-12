const fs = require('fs')
const cargoTomlPath = './src-tauri/Cargo.toml'
const cargoToml = fs.readFileSync(cargoTomlPath, {encoding: 'utf8'})
const matches = cargoToml.match(/version = "(.*?)"/)
if (matches) {
    const version = matches[1]
    const packageJsonPath = './package.json'
    const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, {encoding: 'utf8'}))
    packageJson.version = version
    fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2))

    const tauriConfPath = './src-tauri/tauri.conf.json'
    const tauriConf = JSON.parse(fs.readFileSync(tauriConfPath, {encoding: 'utf8'}))
    tauriConf.package.version = version
    tauriConf.tauri.windows[0].title += ` (v${version})`
    fs.writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2))

    const filesToReplaceVersion = [
        '../.github/chocolatey/factorio-bot.nuspec',
        '../.github/chocolatey/tools/chocolateyinstall.ps1'
    ]
    for (let filePath of filesToReplaceVersion) {
        let content = fs.readFileSync(filePath, {encoding: 'utf8'});
        fs.writeFileSync(filePath, content.replaceAll('__REPLACE_VERSION__', version))
    }
    console.log('PACKAGE_VERSION=' + version)
} else {
    console.error('failed to find version in ', cargoTomlPath)
    process.exit(1)
}