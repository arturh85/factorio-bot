const fs = require('fs')
const targetPath = '../../target/release/'

const version = process.env.PACKAGE_VERSION;

const platforms = {
    'darwin-x86_64': 'bundle/osx/factorio-bot.app.tar.gz.sig',
    'linux-x86_64': 'bundle/appimage/factorio-bot_' + version + '_amd64.AppImage.tar.gz.sig',
    'windows-x86_64': 'bundle/msi/factorio-bot_' + version + '.x64_en-US.msi.zip.sig'
}

let signature = '';

for (let platform of Object.keys(platforms)) {
    const signaturePath = targetPath + platforms[platform]
    console.log('checking', signaturePath);
    if (fs.existsSync(signaturePath)) {
        signature = fs.readFileSync(signaturePath, {encoding: 'utf8'})
        console.log('found', signature);
        break;
    }
}


console.log('::set-output name=signature::' + signature);