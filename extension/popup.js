import { DEFAULTS } from "./rules.js";

const $ = (id) => document.getElementById(id);

const settings = await chrome.storage.local.get(DEFAULTS);
for (const key of Object.keys(DEFAULTS)) {
  const input = $(key);
  const checkbox = input.type === "checkbox";
  input[checkbox ? "checked" : "value"] = settings[key];
  input.addEventListener("change", () => {
    let value = checkbox ? input.checked : input.value;
    if (input.type === "number") value = Math.max(0, Number(value) || 0);
    chrome.storage.local.set({ [key]: value });
  });
}

chrome.runtime.sendMessage({ type: "ping" }).then((reply) => {
  $("status").textContent = reply?.ok
    ? "Connected to the app."
    : reply?.noHost
      ? "Helper not registered. Run: fluxdm-host install"
      : "App is not running. It starts when you send a download.";
});

const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
$("allLinks").addEventListener("click", async () => {
  const reply = await chrome.runtime.sendMessage({ type: "picker", tabId: tab.id });
  if (reply?.ok) window.close();
  else $("allLinks").textContent = reply?.error ?? "Could not read this page";
});

const media = tab ? await chrome.runtime.sendMessage({ type: "list", tabId: tab.id }) : [];
for (const m of media ?? []) {
  const button = document.createElement("button");
  const size = m.size ? ` · ${(m.size / 1048576).toFixed(1)} MB` : "";
  button.textContent = `${m.kind}${size} — ${m.name || m.url.split("?")[0]}`;
  button.addEventListener("click", async () => {
    const reply = await chrome.runtime.sendMessage({ type: "download", tabId: tab.id, url: m.url });
    button.textContent = reply?.ok ? "✓ Sent to FluxDM" : `Failed: ${reply?.error ?? "no reply"}`;
  });
  $("media").append(button);
}
