// Headless browser check: the SPA must work against a real `factorio-bot
// serve` with no Tauri runtime anywhere. Run with `pnpm run e2e` inside the
// nix devShell (it supplies CHROMIUM_BIN).
//
// Prerequisites, both produced by earlier steps of this task:
//   pnpm run build:web
//   cargo build --release --all-features
import {chromium} from 'playwright-core';
import {spawn} from 'node:child_process';
import {mkdirSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..');
const binary = path.join(repoRoot, 'target', 'release', 'factorio-bot');
const webRoot = path.join(repoRoot, 'app', 'dist');
const shots = path.join(here, 'screenshots');
const base = 'http://127.0.0.1:7492';

mkdirSync(shots, {recursive: true});

function fail(message) {
    console.error('FAIL: ' + message);
    process.exitCode = 1;
}

async function waitForHealth(timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const response = await fetch(base + '/api/v1/health');
            if (response.ok) {
                return true;
            }
        } catch {
            // not listening yet
        }
        await new Promise(resolve => setTimeout(resolve, 200));
    }
    return false;
}

const server = spawn(binary, ['serve', '--bind', '127.0.0.1:7492', '--web-root', webRoot], {
    cwd: repoRoot,
    stdio: ['ignore', 'inherit', 'inherit']
});

let browser;
try {
    if (!await waitForHealth(30000)) {
        fail('the server never answered /api/v1/health');
        throw new Error('server did not start');
    }

    browser = await chromium.launch({
        executablePath: process.env.CHROMIUM_BIN || undefined,
        args: ['--no-sandbox']
    });
    const page = await browser.newPage({viewport: {width: 1440, height: 900}});

    const consoleErrors = [];
    const failedApiRequests = [];
    page.on('console', message => {
        if (message.type() === 'error') {
            consoleErrors.push(message.text());
        }
    });
    page.on('response', response => {
        if (response.url().includes('/api/v1/') && response.status() >= 400) {
            failedApiRequests.push(response.status() + ' ' + response.url());
        }
    });

    // 1. The shell loads and there is no Tauri runtime in the page.
    await page.goto(base + '/#/', {waitUntil: 'networkidle'});
    const tauriPresent = await page.evaluate(
        () => typeof window.__TAURI__ !== 'undefined' ||
              typeof window.__TAURI_INTERNALS__ !== 'undefined'
    );
    if (tauriPresent) {
        fail('a Tauri runtime object is present in the page');
    }
    await page.screenshot({path: path.join(shots, 'dashboard.png'), fullPage: true});

    // 2. Settings round-tripped through GET /api/v1/settings: the workspace
    //    path input is populated from the server, which a stubbed or failed
    //    fetch could not do. We read the server's own value independently
    //    (not through the page) and require that exact string to appear
    //    among the rendered input values -- a weaker "any non-empty input"
    //    check would also pass for the unrelated restapiPort input even if
    //    the workspace_path fetch silently failed.
    const settingsResponse = await fetch(base + '/api/v1/settings');
    if (!settingsResponse.ok) {
        fail('GET /api/v1/settings returned ' + settingsResponse.status);
    }
    const settings = await settingsResponse.json();
    const workspacePath = settings?.factorio?.workspace_path;
    if (!workspacePath) {
        fail('server has no workspace_path in its settings: ' + JSON.stringify(settings));
    }

    await page.goto(base + '/#/settings', {waitUntil: 'networkidle'});
    await page.waitForSelector('input', {timeout: 10000});
    const inputValues = await page.$$eval('input', nodes => nodes.map(n => n.value));
    if (workspacePath && !inputValues.includes(workspacePath)) {
        fail('server workspace_path ' + JSON.stringify(workspacePath) +
             ' did not appear among rendered settings inputs: ' + JSON.stringify(inputValues));
    }
    await page.screenshot({path: path.join(shots, 'settings.png'), fullPage: true});

    // 3. The script tree rendered, i.e. GET /api/v1/scripts answered.
    await page.goto(base + '/#/script', {waitUntil: 'networkidle'});
    // `[role="tree"]`, not the old `.p-tree`: that was PrimeVue's own class and
    // it vanished when the tree was rebuilt (plan 6 task 9), turning this check
    // red against a working page. A CSS class is an implementation detail of
    // whichever library renders the tree; the ARIA role is what the tree IS,
    // and it survives the next restyling too.
    //
    // Asserting on the ITEMS rather than the container: an empty `<ul
    // role="tree">` would satisfy a container-only check while telling the
    // user nothing, which is the failure this whole file exists to catch.
    await page.waitForSelector('[role="treeitem"]', {timeout: 10000}).catch(() => {
        fail('the script tree never rendered any items');
    });
    await page.screenshot({path: path.join(shots, 'script.png'), fullPage: true});

    if (consoleErrors.length > 0) {
        fail('console errors: ' + JSON.stringify(consoleErrors, null, 2));
    }
    if (failedApiRequests.length > 0) {
        fail('failing api requests: ' + JSON.stringify(failedApiRequests, null, 2));
    }

    if (process.exitCode !== 1) {
        console.log('OK: browser smoke check passed; screenshots in ' + shots);
    }
} catch (err) {
    fail(err instanceof Error ? err.message : String(err));
} finally {
    if (browser) {
        await browser.close();
    }
    server.kill('SIGTERM');
}
