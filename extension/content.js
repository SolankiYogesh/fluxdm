// Shows a "Download" button when the page has detected media.
(() => {
  addEventListener(
    "mousedown",
    (e) => {
      if (e.isTrusted && e.altKey) chrome.runtime.sendMessage({ type: "bypass" }).catch(() => {});
    },
    true,
  );
  if (window !== top) return;

  let host = null;
  let dismissed = false;

  const el = (tag, text, className) => {
    const node = document.createElement(tag);
    if (text) node.textContent = text;
    if (className) node.className = className;
    return node;
  };

  function build() {
    host = el("div");
    const root = host.attachShadow({ mode: "closed" });
    const style = el("style", `
      .box{position:fixed;top:12px;right:12px;z-index:2147483647;font:13px/1.4 Geist,Inter,system-ui,sans-serif;color:#fafafa}
      button{font:inherit;color:inherit;border:0;cursor:pointer;background:none;transition:background-color .15s}
      button:focus-visible{outline:2px solid #a1a1aa;outline-offset:-2px}
      .bar{display:flex;margin-left:auto;width:max-content;background:#09090b;border:1px solid #3f3f46;border-radius:10px;padding:3px;box-shadow:0 8px 24px #0008}
      .bar button{padding:7px 11px;border-radius:7px;font-weight:500}
      .bar button:hover{background:#27272a}
      .list{display:none;background:#09090b;border:1px solid #3f3f46;border-radius:10px;padding:4px;margin-top:6px;max-width:380px;max-height:50vh;overflow:auto;box-shadow:0 12px 32px #0009}
      .list.open{display:block}
      .list button{display:block;width:100%;text-align:left;padding:8px 10px;border-radius:7px;overflow-wrap:anywhere}
      .list button:hover{background:#27272a}`);
    const box = el("div", "", "box");
    const bar = el("div", "", "bar");
    const main = el("button", "⬇ Download media");
    const close = el("button", "✕");
    close.title = "Hide on this page";
    const list = el("div", "", "list");
    bar.append(main, close);
    box.append(bar, list);
    root.append(style, box);

    close.addEventListener("click", () => {
      dismissed = true;
      remove();
    });
    main.addEventListener("click", async (event) => {
      if (!event.isTrusted) return;
      if (list.classList.toggle("open")) {
        const media = await chrome.runtime.sendMessage({ type: "list" });
        list.replaceChildren(
          ...media.map((m) => {
            const size = m.size ? ` · ${(m.size / 1048576).toFixed(1)} MB` : "";
            const row = el("button", `${m.kind}${size} — ${m.name || m.url.split("?")[0]}`);
            row.addEventListener("click", async (e) => {
              // Pages must not be able to start downloads with synthetic clicks.
              if (!e.isTrusted) return;
              const reply = await chrome.runtime.sendMessage({ type: "download", url: m.url });
              row.textContent = reply?.ok ? "✓ Sent to FluxDM" : `Failed: ${reply?.error ?? "no reply"}`;
            });
            return row;
          }),
        );
      }
    });
    document.documentElement.append(host);
  }

  function remove() {
    host?.remove();
    host = null;
  }

  chrome.runtime.onMessage.addListener((message) => {
    if (message.type !== "media-count") return;
    if (message.count === 0) {
      dismissed = false;
      remove();
      return;
    }
    chrome.storage.local.get({ videoButton: true }).then(({ videoButton }) => {
      if (videoButton && !dismissed && !host) build();
    });
  });
})();
