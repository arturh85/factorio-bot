const fs = require('fs')
const crypto = require('crypto')
const algorithm = 'sha256', shasum = crypto.createHash(algorithm)
const cargoTomlPath = './src-tauri/Cargo.toml'
const cargoToml = fs.readFileSync(cargoTomlPath, {encoding: 'utf8'})
const matches = cargoToml.match(/version = "(.*?)"/)
if (matches) {
    const version = matches[1]
    const filename = `src-tauri/target/release/bundle/msi/factorio-bot_${version}_x64.msi`, s = fs.ReadStream(filename)
    s.on('data', function (data) {
        shasum.update(data)
    });
    s.on('end', function () {
        console.info('hashing completed');

        const checksum = shasum.digest('hex')
        const filesToReplaceChecksum = [
            '../.github/chocolatey/tools/chocolateyinstall.ps1'
        ]
        for (let filePath of filesToReplaceChecksum) {
            let content = fs.readFileSync(filePath, {encoding: 'utf8'});
            fs.writeFileSync(filePath, content.replaceAll('__REPLACE_CHECKSUM__', checksum))
            console.info('replaced checksum in ', filePath);
        }
    })
} else {
    console.error('no version found');
}

