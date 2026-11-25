// Regenerate every raster icon from assets/icon.svg.
// Needs Playwright: npm i --no-save playwright && npx playwright install chromium
// The .icns file is only rebuilt on macOS.
import { chromium } from "playwright";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const root = path.resolve(process.argv[2] ?? ".");
const svg = fs.readFileSync(path.join(root, "assets/icon.svg"), "utf8");
const browser = await chromium.launch();
const page = await browser.newPage();

// `margin` keeps the macOS safe area; elsewhere the tile fills the canvas.
async function render(size, margin) {
  const view = margin ? "0 0 1024 1024" : "92 92 840 840";
  const sized = svg.replace(/width="1024" height="1024" viewBox="0 0 1024 1024"/, `width="${size}" height="${size}" viewBox="${view}"`);
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(`<style>html,body{margin:0}svg{display:block}</style>${sized}`);
  return page.screenshot({ omitBackground: true });
}

const write = (file, data) => {
  fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
  fs.writeFileSync(path.join(root, file), data);
};

write("crates/app/assets/icon.png", await render(256, false));
for (const size of [16, 48, 128]) write(`extension/icons/${size}.png`, await render(size, false));
write("assets/icon-512.png", await render(512, false));

// Windows .ico holding PNG images.
const sizes = [16, 24, 32, 48, 64, 128, 256];
const images = [];
for (const size of sizes) images.push(await render(size, false));
const header = Buffer.alloc(6 + 16 * sizes.length);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(sizes.length, 4);
let offset = header.length;
sizes.forEach((size, i) => {
  const at = 6 + 16 * i;
  header.writeUInt8(size % 256, at);
  header.writeUInt8(size % 256, at + 1);
  header.writeUInt16LE(1, at + 4);
  header.writeUInt16LE(32, at + 6);
  header.writeUInt32LE(images[i].length, at + 8);
  header.writeUInt32LE(offset, at + 12);
  offset += images[i].length;
});
write("assets/icon.ico", Buffer.concat([header, ...images]));

if (process.platform === "darwin") {
  const set = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "icons-")), "icon.iconset");
  fs.mkdirSync(set);
  for (const size of [16, 32, 128, 256, 512]) {
    fs.writeFileSync(path.join(set, `icon_${size}x${size}.png`), await render(size, true));
    fs.writeFileSync(path.join(set, `icon_${size}x${size}@2x.png`), await render(size * 2, true));
  }
  execFileSync("iconutil", ["-c", "icns", set, "-o", path.join(root, "assets/icon.icns")]);
}
await browser.close();
