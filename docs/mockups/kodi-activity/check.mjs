// Run from the repository root with the prototype server on port 8091.
import assert from 'node:assert/strict';
import {chromium} from '../../../build/webui-review/node_modules/playwright/index.mjs';
const browser = await chromium.launch();
try {
    const page = await browser.newPage({viewport: {width: 1600, height: 1300}});
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(process.env.MOCKUP_URL || 'http://127.0.0.1:8091');
    await page.evaluate(() => { window.freezeSim = true; });
    await page.evaluate(() => document.fonts.ready);
    for (const id of ['a', 'b', 'c']) {
        const remote = page.locator(`#remote-${id}`);
        await remote.getByRole('button', {name: 'Pause', exact: true}).click();
        await remote.getByRole('button', {name: 'Play', exact: true}).waitFor();
        await remote.getByRole('button', {name: 'Play', exact: true}).click();
        const initial = Number(await remote.locator('.seek').inputValue());
        await remote.getByRole('button', {name: 'Back 10 seconds'}).click();
        assert.equal(Number(await remote.locator('.seek').inputValue()), initial - 10);
        if (id === 'c') await remote.getByRole('button', {name: 'More controls'}).click();
        await remote.getByRole('button', {name: 'Chapters', exact: true}).click();
        const dialog = remote.getByRole('dialog');
        const list = dialog.locator('.chapter-list');
        assert(await list.evaluate(el => el.scrollHeight <= el.clientHeight));
        await dialog.getByRole('button', {name: 'Chapter 5: One more chance, 7:16'}).click();
        assert.equal(Number(await remote.locator('.seek').inputValue()), 436);
        if (id === 'c') await remote.getByRole('button', {name: 'More controls'}).click();
        await remote.getByRole('button', {name: 'Audio', exact: true}).click();
        await remote.getByRole('button', {name: 'English · Stereo'}).click();
        assert(await remote.getByRole('button', {name: 'English · Stereo'}).evaluate(el => el.classList.contains('current')));
        await page.keyboard.press('Escape');
        assert.equal(await remote.getByRole('dialog').count(), 0);
        await remote.getByRole('button', {name: 'Return to room'}).click();
        await remote.getByText('Playback continues on your TV.').waitFor();
        await remote.getByRole('button', {name: 'Return to Watch Kodi'}).click();
        await remote.getByRole('button', {name: 'Pause', exact: true}).waitFor();
    }
    for (const fixture of ['missing', 'idle', 'offline', 'live', 'legacy']) {
        await page.locator('#fixture').selectOption(fixture);
        if (fixture === 'missing') assert.equal(await page.locator('.fallback-title').count(), 3);
        if (fixture === 'idle') {
            await page.locator('#remote-a').getByRole('button', {name: 'TV controls'}).click();
            await page.keyboard.press('ArrowUp');
            await page.locator('#remote-a').getByText('Demo · Up sent to Kodi').waitFor();
            await page.keyboard.press('Escape');
        }
        if (fixture === 'offline') assert.equal(await page.getByRole('button', {name: 'Retry connection'}).count(), 3);
        if (fixture === 'live') assert.equal(await page.locator('.seek:disabled').count(), 3);
        if (fixture === 'legacy') {
            await page.locator('#remote-a').getByRole('button', {name: 'Chapters', exact: true}).click();
            await page.getByText(/does not provide a chapter list/).waitFor();
        }
    }
    await page.locator('#fixture').selectOption('playing');
    for (const id of ['a', 'b', 'c']) await page.locator(`#remote-${id}`).screenshot({path: `docs/mockups/kodi-activity/${id}.png`});
    await page.locator('#remote-a').getByRole('button', {name: 'Chapters', exact: true}).click();
    await page.waitForTimeout(250);
    await page.locator('#remote-a').screenshot({path: 'docs/mockups/kodi-activity/chapters.png'});
    await page.keyboard.press('Escape');
    await page.setViewportSize({width: 390, height: 844});
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.screenshot({path: 'build/kodi-mockup-mobile.png', fullPage: true});
    assert.deepEqual(errors, []);
    console.log('PASS: playback, chapter selection, whole chapter rows, audio, Back, idle TV controls, unavailable states and mobile layout.');
} finally {
    await browser.close();
}
