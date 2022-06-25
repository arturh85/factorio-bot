const fs = require('fs')
const updaterPath = '../../updates/'
const targetPath = '../../target/release/'

// https://tauri.app/v1/guides/distribution/updater/#update-server-json-format

const version = process.env.PACKAGE_VERSION;
const releaseBody = process.env.RELEASE_BODY;

// Note that each platform key is in the OS-ARCH format, where OS is one of linux, darwin or windows, and ARCH is one of x86_64, aarch64, i686 or armv7.

const platforms = {
    'darwin-x86_64': 'factorio-bot.app.tar.gz',
    'linux-x86_64': 'factorio-bot_' + version + '_amd64.AppImage.tar.gz',
    'windows-x86_64': 'factorio-bot_' + version + '_x64_en-US.msi.zip'
}

const sigs = {
    'darwin-x86_64': `bundle/macos/${platforms['darwin-x86_64']}.sig`,
    'linux-x86_64': `bundle/appimage/${platforms['linux-x86_64']}.sig`,
    'windows-x86_64': `bundle/msi/${platforms['windows-x86_64']}.sig`
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

