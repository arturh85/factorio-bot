const fs = require('fs')
const updaterPath = '../../public/updates/'

// https://tauri.app/v1/guides/distribution/updater/#update-server-json-format

const version = process.env.PACKAGE_VERSION;
const releaseBody = process.argv[2];

const platforms = {
    'darwin-x86_64': 'factorio-bot.app.tar.gz',
    'linux-x86_64': 'factorio-bot_' + version + '_amd64.AppImage.tar.gz',
    'windows-x86_64': 'factorio-bot_' + version + '.x64.msi.zip'
}

for (let platform of Object.keys(platforms)) {
    const urlFilename = platforms[platform];
    const platformPath = updaterPath + platform
    const nowStr = new Date().toISOString();
    const platformJson = {
        'version': 'v' + version,
        'notes': releaseBody,
        'pub_date': nowStr,
        'platforms': {
            [platform]: {
                'signature': '',
                'url': 'https://github.com/arturh85/factorio-bot-tauri/releases/download/v' + version + '/' + urlFilename
            }
        }
    };
    fs.writeFileSync(platformPath, JSON.stringify(platformJson, null, 2))
}

