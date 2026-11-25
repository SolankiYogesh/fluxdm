import { DEFAULTS, shouldCapture, classifyMedia, suggestedName, linkType } from "./rules.js";

const HOST = "com.fluxdm.host";
const MAX_MEDIA = 30;

const getSettings = () => chrome.storage.local.get(DEFAULTS);
const send = (message) => chrome.runtime.sendNativeMessage(HOST, message);

async function cookieHeader(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => (c.name ? `${c.name}=${c.value}` : c.value)).join("; ") || null;
  } catch {
    return null;
  }
}

async function toItem(url, referrer, filename = null) {
  return {
    url,
    referrer: referrer || null,
    cookie: await cookieHeader(url),
    userAgent: navigator.userAgent,
    filename,
  };
}

function flash(text, color) {
  chrome.action.setBadgeBackgroundColor({ color });
  chrome.action.setBadgeText({ text });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 4000);
}

async function deliver(items, confirmed = false) {
  if (!items.length) return { ok: false, error: "no downloadable links" };
  try {
    const reply = await send({ cmd: "add", items, confirmed });
    if (!reply?.ok) flash("!", "#c0392b");
    return reply ?? { ok: false, error: "no reply" };
  } catch (e) {
    flash("!", "#c0392b");
    return { ok: false, error: e.message };
  }
}

// Requests made with POST cannot be replayed from an address alone, so their
// downloads stay in the browser.
const recentPosts = new Map();
chrome.webRequest.onBeforeRequest.addListener(
  (d) => {
    if (d.method !== "POST") return;
    const now = Date.now();
    for (const [url, at] of recentPosts) if (now - at > 60000) recentPosts.delete(url);
    recentPosts.set(d.url, now);
  },
  { urls: ["<all_urls>"], types: ["main_frame", "sub_frame"] },
);

// Alt+click means "let the browser handle this one".
let bypassUntil = 0;

// Take over a browser download. The browser's copy is cancelled only after
// FluxDM confirms it accepted the job, so a failure never loses a download.
async function takeOver(item) {
  const settings = await getSettings();
  const url = item.finalUrl || item.url;
  if (Date.now() < bypassUntil || recentPosts.has(url) || recentPosts.has(item.url)) return;
  if (!shouldCapture(item, settings)) return;
  await chrome.downloads.pause(item.id).catch(() => {});
  const name = item.filename ? item.filename.split(/[\\/]/).pop() : null;
  const reply = await deliver([await toItem(url, item.referrer, name)]);
  if (reply.ok) {
    await chrome.downloads.cancel(item.id).catch(() => {});
    await chrome.downloads.erase({ id: item.id }).catch(() => {});
  } else {
    await chrome.downloads.resume(item.id).catch(() => {});
  }
}

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  takeOver(item)
    .catch((e) => console.warn("FluxDM:", e))
    .finally(() => {
      try {
        suggest();
      } catch {
        // The download was already cancelled.
      }
    });
  return true;
});

// ---- Media detection -------------------------------------------------------

const mediaKey = (tabId) => `media:${tabId}`;
// Storage updates are serialised so simultaneous responses cannot drop entries.
let queue = Promise.resolve();
const enqueue = (job) => (queue = queue.then(job).catch((e) => console.warn("FluxDM:", e)));

async function listMedia(tabId) {
  const key = mediaKey(tabId);
  return (await chrome.storage.session.get(key))[key] ?? [];
}

async function setMedia(tabId, list) {
  await chrome.storage.session.set({ [mediaKey(tabId)]: list });
  chrome.action.setBadgeBackgroundColor({ tabId, color: "#1f7a55" }).catch(() => {});
  chrome.action.setBadgeText({ tabId, text: list.length ? String(list.length) : "" }).catch(() => {});
  chrome.tabs.sendMessage(tabId, { type: "media-count", count: list.length }).catch(() => {});
}

chrome.webRequest.onHeadersReceived.addListener(
  (d) => {
    if (d.tabId < 0 || d.method !== "GET" || ![200, 206].includes(d.statusCode)) return;
    const header = (name) => d.responseHeaders?.find((h) => h.name.toLowerCase() === name)?.value ?? "";
    const total = header("content-range").split("/")[1] || header("content-length");
    const found = classifyMedia(
      { url: d.url, contentType: header("content-type"), size: Number(total) || 0 },
      d.initiator || "",
    );
    if (!found) return;
    enqueue(async () => {
      const list = await listMedia(d.tabId);
      if (list.some((m) => m.url === d.url) || list.length >= MAX_MEDIA) return;
      const tab = await chrome.tabs.get(d.tabId);
      if (tab.incognito || !classifyMedia({ url: d.url, contentType: header("content-type") }, tab.url)) return;
      list.push({
        url: d.url,
        kind: found.kind,
        size: found.size,
        referrer: tab.url,
        name: suggestedName(tab.title, d.url, found.kind),
      });
      await setMedia(d.tabId, list);
    });
  },
  { urls: ["http://*/*", "https://*/*"], types: ["media", "xmlhttprequest", "object", "other"] },
  ["responseHeaders"],
);

chrome.webRequest.onBeforeRequest.addListener(
  (d) => {
    if (d.tabId >= 0) enqueue(() => setMedia(d.tabId, []));
  },
  { urls: ["<all_urls>"], types: ["main_frame"] },
);

chrome.tabs.onRemoved.addListener((tabId) => enqueue(() => chrome.storage.session.remove(mediaKey(tabId))));

// ---- Context menu ----------------------------------------------------------

chrome.runtime.onInstalled.addListener(async () => {
  await chrome.contextMenus.removeAll();
  const menus = [
    ["one", "Download with FluxDM", ["link", "video", "audio", "image"]],
    ["selected", "Download selected links with FluxDM", ["selection"]],
    ["all", "Download all links with FluxDM", ["page", "frame", "link"]],
  ];
  for (const [id, title, contexts] of menus) chrome.contextMenus.create({ id, title, contexts });
});

// Runs inside the page, so it must be self-contained.
function collectLinks(selectionOnly) {
  const selection = getSelection();
  const found = new Map();
  const nodes = document.querySelectorAll("a[href], area[href], img[src], video[src], audio[src], source[src]");
  for (const node of nodes) {
    if (selectionOnly && !selection?.containsNode(node, true)) continue;
    const url = String(node.href || node.currentSrc || node.src || "").split("#")[0];
    if (!/^(https?|ftp):/i.test(url) || found.has(url)) continue;
    const text = (node.innerText || node.alt || node.title || "").trim().replace(/\s+/g, " ");
    found.set(url, text.slice(0, 120));
  }
  return [...found].map(([url, text]) => ({ url, text }));
}

const MAX_LINKS = 3000;
const pickerKey = (id) => `picker:${id}`;

// Collect the page's links and let the user tick the ones wanted.
async function openPicker(tab, frameId, selectionOnly) {
  const target = frameId === undefined
    ? { tabId: tab.id, allFrames: !selectionOnly }
    : { tabId: tab.id, frameIds: [frameId] };
  let results;
  try {
    results = await chrome.scripting.executeScript({ target, func: collectLinks, args: [selectionOnly] });
  } catch (e) {
    flash("!", "#c0392b");
    return { ok: false, error: e.message };
  }
  const byUrl = new Map(results.flatMap((r) => r.result ?? []).map((l) => [l.url, l]));
  const links = [...byUrl.values()].slice(0, MAX_LINKS).map((l) => ({ ...l, type: linkType(l.url) }));
  if (!links.length) {
    flash("0", "#c0392b");
    return { ok: false, error: "no links found" };
  }
  const id = crypto.randomUUID();
  await chrome.storage.session.set({ [pickerKey(id)]: { page: tab.url, selectionOnly, links } });
  await chrome.windows.create({ url: `links.html?id=${id}`, type: "popup", width: 900, height: 640 });
  return { ok: true };
}

async function downloadPicked(id, urls) {
  const data = (await chrome.storage.session.get(pickerKey(id)))[pickerKey(id)];
  if (!data) return { ok: false, error: "this list has expired; open it again" };
  const known = new Set(data.links.map((l) => l.url));
  const chosen = urls.filter((u) => known.has(u));
  let accepted = 0;
  // Small batches keep each native message well under the size limit.
  for (let i = 0; i < chosen.length; i += 50) {
    const items = await Promise.all(chosen.slice(i, i + 50).map((u) => toItem(u, data.page)));
    const reply = await deliver(items, true);
    accepted += reply.accepted ?? 0;
    if (!reply.ok) return { ok: false, accepted, error: reply.error };
  }
  return { ok: accepted > 0, accepted };
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "one") {
    const url = info.linkUrl || info.srcUrl;
    if (url && /^(https?|ftp):/i.test(url)) await deliver([await toItem(url, info.frameUrl || tab?.url)]);
    else flash("!", "#c0392b");
  } else if (tab) {
    const selected = info.menuItemId === "selected";
    // From the page menu, search every frame; a selection lives in one frame.
    await openPicker(tab, selected ? (info.frameId ?? 0) : undefined, selected);
  }
});

// ---- Messages from the popup and content script ----------------------------

async function handle(message, sender) {
  if (message.type === "bypass") {
    bypassUntil = Date.now() + 5000;
    return { ok: true };
  }
  // Only extension pages may name a tab; a content script gets its own.
  const ownPage = sender.url?.startsWith(chrome.runtime.getURL(""));
  const tabId = ownPage ? (message.tabId ?? sender.tab?.id) : sender.tab?.id;
  if (message.type === "list") return listMedia(tabId);
  if (message.type === "picker" && ownPage) {
    return openPicker(await chrome.tabs.get(message.tabId), undefined, false);
  }
  if (message.type === "download-picked" && ownPage) return downloadPicked(message.id, message.urls);
  if (message.type === "download") {
    const media = (await listMedia(tabId)).find((m) => m.url === message.url);
    if (!media) return { ok: false, error: "that media is no longer listed" };
    return deliver([await toItem(media.url, media.referrer, media.name)]);
  }
  if (message.type === "ping") {
    try {
      return await send({ cmd: "ping" });
    } catch (e) {
      return { ok: false, error: e.message, noHost: true };
    }
  }
  return null;
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  // Only this extension's own pages and content scripts may ask.
  if (sender.id !== chrome.runtime.id) return false;
  handle(message, sender).then(respond, (e) => respond({ ok: false, error: e.message }));
  return true;
});
