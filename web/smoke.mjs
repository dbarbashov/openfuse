// Run against a locally served build or OPENFUSE_WEB_URL. Tests use synthetic files only.
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
const browser = await puppeteer.launch({
  ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}),
  headless: true, args: ['--enable-unsafe-swiftshader', '--no-sandbox'],
});
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'openfuse-web-test-'));
try {
  const page = await browser.newPage();
  await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 1 });
  const errors = [], requests = [];
  page.on('pageerror', e => errors.push(String(e)));
  page.on('request', req => requests.push(req.url()));
  page.on('dialog', dialog => dialog.accept());
  await page.evaluateOnNewDocument(() => {
    window.testDownloads = [];
    const original = URL.createObjectURL;
    URL.createObjectURL = function(blob) {
      blob.text().then(text => window.testDownloads.push(text));
      return original.call(this, blob);
    };
  });
  await page.goto(process.env.OPENFUSE_WEB_URL || 'http://127.0.0.1:8766/', { waitUntil: 'networkidle0' });
  await page.waitForFunction(() => !document.querySelector('#download').disabled, { timeout: 60000 });
  async function download() {
    const before = await page.evaluate(() => window.testDownloads.length);
    await page.click('#download');
    await page.waitForFunction(n => window.testDownloads.length > n, {}, before);
    return page.evaluate(() => window.testDownloads.at(-1));
  }
  const baseline = await download();
  assert.match(baseline, /Warm room/);
  // Actual canvas gesture changes the amplifier volume in the shared Rust UI.
  const canvas = await page.$eval("#canvas", el => ({x: el.getBoundingClientRect().x, y: el.getBoundingClientRect().y}));
  await page.mouse.click(canvas.x + 220, canvas.y + 483);
  await new Promise(resolve => setTimeout(resolve, 250));
  const edited = await download();
  assert.notEqual(edited, baseline, 'Canvas slider must change exported XML');
  await page.click('#undo'); assert.equal(await download(), baseline);
  await page.click('#redo'); assert.equal(await download(), edited);
  const fixture = path.join(temporary, 'roundtrip.fuse');
  const withUnknown = edited.replace('</Preset>', '<IndependentTest future="retained"/></Preset>');
  await fs.writeFile(fixture, withUnknown);
  await (await page.$('#file')).uploadFile(fixture);
  await page.waitForFunction(() => document.querySelector('#status').textContent.includes('roundtrip.fuse'));
  assert.equal(await download(), withUnknown, 'Unknown XML must roundtrip unchanged');
  const malformed = path.join(temporary, 'malformed.fuse');
  await fs.writeFile(malformed, '<Preset broken');
  await (await page.$('#file')).uploadFile(malformed);
  await page.waitForFunction(() => document.querySelector('#error').textContent.length > 0);
  assert.equal(await download(), withUnknown, 'Rejected input must preserve the current document');
  const families = await page.$$eval('#product option', options => options.map(o => o.value));
  assert.equal(families.length, 7);
  for (const product of families) {
    await page.select('#product', product); await page.click('#new');
    assert.match(await download(), new RegExp(`ProductId="${product}"`));
  }
  await page.click('#example');
  await page.mouse.click(canvas.x + 200, canvas.y + 144); // Expression inspector
  await page.mouse.click(canvas.x + 320, canvas.y + 144); // Details inspector
  await page.mouse.click(canvas.x + 80, canvas.y + 144); // Sound inspector
  await page.click('#theme');
  await new Promise(resolve => setTimeout(resolve, 300));
  if (process.env.SCREENSHOT_PATH) await page.screenshot({ path: process.env.SCREENSHOT_PATH });
  assert.equal(await page.$eval('#error', el => el.textContent), '');
  assert.deepEqual(errors, []);
  const origin = new URL(page.url()).origin;
  assert.deepEqual(requests.filter(url => !url.startsWith(origin) && !url.startsWith('blob:') && !url.startsWith('data:')), [], 'No third-party requests or preset uploads');
  await page.setViewport({width: 940, height: 680});
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  console.log('PASS: WebGL startup, canvas edit, undo/redo, exact XML download, unknown XML, rejected import, seven families, inspectors, theme, viewport, local-only requests.');
} finally {
  await browser.close(); await fs.rm(temporary, { recursive: true, force: true });
}
