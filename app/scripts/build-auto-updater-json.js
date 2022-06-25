const fs = require('fs')
const updaterPath = '../../updates/'
const targetPath = '../../target/release/'

// https://tauri.app/v1/guides/distribution/updater/#update-server-json-format

const version = process.env.PACKAGE_VERSION;
const releaseBody = process.env.RELEASE_BODY;

console.log('DEBUG ENV', process.env)

const platforms = {
    'darwin-x86_64': 'factorio-bot.app.tar.gz',
    'linux-x86_64': 'factorio-bot_' + version + '_amd64.AppImage.tar.gz',
    'windows-x86_64': 'factorio-bot_' + version + '.x64.msi.zip'
}

const sigs = {
    'darwin-x86_64': 'bundle/osx/factorio-bot.app.tar.gz.sig',
    'linux-x86_64': 'bundle/appimage/factorio-bot_' + version + '_amd64.AppImage.tar.gz.sig',
    'windows-x86_64': 'bundle/msi/factorio-bot_' + version + '.x64_en-US.msi.zip.sig'
}

for (let platform of Object.keys(platforms)) {
    const urlFilename = platforms[platform];
    const signaturePath = targetPath + sigs[platform];

    console.log('checking', signaturePath)
    if (fs.existsSync(signaturePath)) {
        const signature = fs.readFileSync(signaturePath, {encoding: 'utf8'})
        const platformPath = updaterPath + platform + '.json'
        const nowStr = new Date().toISOString();
        const platformJson = {
            'version': 'v' + version,
            'notes': releaseBody,
            'pub_date': nowStr,
            'platforms': {
                [platform]: {
                    'signature': signature,
                    'url': 'https://github.com/arturh85/factorio-bot-tauri/releases/download/v' + version + '/' + urlFilename
                }
            }
        };
        console.log('write to ', platformPath, platformJson);
        fs.writeFileSync(platformPath, JSON.stringify(platformJson, null, 2))
        break;
    }
}

