const fs = require('fs')
const updaterPath = '../../public/updates/'

// https://tauri.app/v1/guides/distribution/updater/#update-server-json-format

const platforms = [
    'darwin-x86_64',
    'linux-x86_64',
    'windows-x86_64'
]

let obj = null;

for (let platform of platforms) {
    const platformJsonPath = updaterPath + platform + '.json';
    let json = JSON.parse(fs.readFileSync(platformJsonPath, {encoding: 'utf8'}))
    if (!obj) {
        obj = json;
    } else {
        obj['platforms'][platform] = json['platforms'][platform];
    }
}

fs.writeFileSync(updaterPath + 'all.json', JSON.stringify(obj, null, 2));
