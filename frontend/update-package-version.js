const fs = require('fs')
const cargoTomlPath = './src-tauri/Cargo.toml'
const cargoToml = fs.readFileSync(cargoTomlPath, {encoding: 'utf8'})
const matches = cargoToml.match(/version = "(.*?)"/)
if (matches) {
    const packageJsonPath = './package.json'
    const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, {encoding: 'utf8'}))
    packageJson.version = matches[1]
    fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2))
} else {
    console.error('failed to find version in ', cargoTomlPath)
    process.exit(1)
}