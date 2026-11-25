// Pure decision logic, kept apart from browser APIs so it can be tested.

export const DEFAULTS = {
  enabled: true,
  videoButton: true,
  captureAll: false,
  minSizeKB: 0,
  extensions:
    "3gp 7z aac aif apk arj asf avi bin bz2 deb dmg doc docx exe flac gz gzip img iso lzh m4a m4v mkv mov " +
    "mp3 mp4 mpa mpe mpeg mpg msi msix msu ogg ogv opus pdf pkg ppt pptx qt rar rpm sit tar tgz tif tiff " +
    "torrent ts wav webm wma wmv xls xlsx xz z zip zst",
  excludedSites: "",
};

export const words = (text) => text.toLowerCase().split(/[\s,]+/).filter(Boolean);

export function extensionOf(name) {
  const base = String(name || "").split(/[\\/]/).pop();
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : "";
}

export function pathOf(url) {
  try {
    return decodeURIComponent(new URL(url).pathname);
  } catch {
    return "";
  }
}

export function isExcluded(url, patterns) {
  let host;
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return false;
  }
  return words(patterns).some((p) => {
    const bare = p.replace(/^\*\./, "");
    return host === bare || host.endsWith("." + bare);
  });
}

const FILE_TYPES =
  /^(video\/.+|audio\/.+|application\/(octet-stream|zip|gzip|pdf|msword|x-(7z-compressed|rar-compressed|tar|bzip2|xz|msdownload|msdos-program|msi|apple-diskimage|iso9660-image|bittorrent)|vnd\.(rar|android\.package-archive|microsoft\.portable-executable|openxmlformats-officedocument\..+|ms-.+)))$/i;

const IMAGE_TYPES = "jpg jpeg png gif webp svg bmp";

/** Sort a link for the picker window. */
export function linkType(url) {
  const ext = extensionOf(pathOf(url));
  const groups = {
    Video: "mp4 mkv webm avi mov flv m4v wmv mpg mpeg ts m3u8 3gp ogv",
    Music: "mp3 m4a flac wav ogg opus aac wma aif",
    Images: IMAGE_TYPES,
    Documents: "pdf doc docx xls xlsx ppt pptx txt csv epub srt vtt",
    Compressed: "zip rar 7z gz tgz bz2 xz zst tar",
    Programs: "exe msi apk dmg pkg deb rpm iso img bin appimage torrent",
  };
  return Object.keys(groups).find((g) => words(groups[g]).includes(ext)) ?? "Other";
}

export function shouldCapture(item, settings) {
  if (!settings.enabled || item.incognito || item.byExtensionId) return false;
  const url = item.finalUrl || item.url;
  if (!/^(https?|ftp):/i.test(url)) return false;
  if (isExcluded(url, settings.excludedSites)) return false;
  if (item.referrer && isExcluded(item.referrer, settings.excludedSites)) return false;
  if (item.totalBytes > 0 && item.totalBytes < settings.minSizeKB * 1024) return false;
  if (settings.captureAll) return true;
  // Addresses like /download?id=7 carry no extension; the server's type does.
  if (FILE_TYPES.test(String(item.mime || "").split(";")[0].trim())) return true;
  const wanted = new Set(words(settings.extensions));
  return wanted.has(extensionOf(item.filename)) || wanted.has(extensionOf(pathOf(url)));
}

// Sites whose delivery formats or terms this extension deliberately skips.
const UNSUPPORTED_MEDIA = "youtube.com youtube-nocookie.com googlevideo.com youtu.be";

/** Classify a response as downloadable media, or return null. */
export function classifyMedia({ url, contentType = "", size = 0 }, pageUrl = "") {
  if (isExcluded(url, UNSUPPORTED_MEDIA) || isExcluded(pageUrl, UNSUPPORTED_MEDIA)) return null;
  const type = contentType.split(";")[0].trim().toLowerCase();
  const ext = extensionOf(pathOf(url));
  if (ext === "m3u8" || type.includes("mpegurl")) return { kind: "HLS stream", size: 0 };
  // Individual stream fragments are useless on their own.
  if (["ts", "m4s"].includes(ext) || type === "video/mp2t" || type === "video/iso.segment") return null;
  const media = /^(video|audio)\//.test(type) || ["mp4", "webm", "mkv", "mov", "flv", "mp3", "m4a"].includes(ext);
  if (!media || /^(text|image)\//.test(type)) return null;
  if (size > 0 && size < 256 * 1024) return null;
  return { kind: type.startsWith("audio/") || ["mp3", "m4a"].includes(ext) ? "Audio" : "Video", size };
}

const RESERVED = /[<>:"/\\|?*\u0000-\u001f]+/g;

export function suggestedName(pageTitle, url, kind) {
  const fromUrl = pathOf(url).split("/").pop() || "";
  if (kind !== "HLS stream" && fromUrl && !/^(videoplayback|index|play)\b/i.test(fromUrl)) return fromUrl;
  const title = String(pageTitle || "").replace(RESERVED, " ").trim().slice(0, 100);
  if (!title) return null;
  const ext = kind === "HLS stream" ? "m3u8" : extensionOf(fromUrl) || (kind === "Audio" ? "mp3" : "mp4");
  return `${title}.${ext}`;
}
