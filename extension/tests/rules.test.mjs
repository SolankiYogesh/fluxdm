import test from "node:test";
import assert from "node:assert/strict";
import { DEFAULTS, shouldCapture, classifyMedia, suggestedName, isExcluded } from "../rules.js";

test("captures listed file types only", () => {
  assert.ok(shouldCapture({ url: "https://a.com/x.zip" }, DEFAULTS));
  assert.ok(shouldCapture({ url: "https://a.com/get?id=1", filename: "setup.EXE" }, DEFAULTS));
  assert.ok(!shouldCapture({ url: "https://a.com/page.html" }, DEFAULTS));
  assert.ok(!shouldCapture({ url: "blob:https://a.com/1", filename: "x.zip" }, DEFAULTS));
});

test("leaves private, small, excluded and disabled downloads alone", () => {
  const item = { url: "https://dl.example.com/x.zip", totalBytes: 100 };
  assert.ok(!shouldCapture({ ...item, incognito: true }, DEFAULTS));
  assert.ok(!shouldCapture(item, { ...DEFAULTS, minSizeKB: 10 }));
  assert.ok(!shouldCapture(item, { ...DEFAULTS, excludedSites: "*.example.com" }));
  assert.ok(!shouldCapture(item, { ...DEFAULTS, enabled: false }));
  assert.ok(!isExcluded("https://notexample.com/", "example.com"));
});

test("recognises media but not fragments or unsupported sites", () => {
  assert.equal(classifyMedia({ url: "https://c.com/a/master.m3u8?t=1" }).kind, "HLS stream");
  assert.equal(classifyMedia({ url: "https://c.com/v", contentType: "video/mp4", size: 5e6 }).kind, "Video");
  assert.equal(classifyMedia({ url: "https://c.com/seg1.ts", contentType: "video/mp2t", size: 5e6 }), null);
  assert.equal(classifyMedia({ url: "https://c.com/a.mp4", size: 1000 }), null);
  assert.equal(classifyMedia({ url: "https://r.googlevideo.com/videoplayback", contentType: "video/mp4" }), null);
});

test("names streams after the page", () => {
  assert.equal(suggestedName("My: Talk / 2", "https://c.com/index.m3u8", "HLS stream"), "My  Talk   2.m3u8");
  assert.equal(suggestedName("T", "https://c.com/clip.mp4", "Video"), "clip.mp4");
});

test("captures by server type when the address has no extension", () => {
  assert.ok(shouldCapture({ url: "https://a.com/get?id=7", mime: "application/octet-stream" }, DEFAULTS));
  assert.ok(!shouldCapture({ url: "https://a.com/get?id=7", mime: "text/html" }, DEFAULTS));
  assert.ok(shouldCapture({ url: "https://a.com/x.weird" }, { ...DEFAULTS, captureAll: true }));
});
