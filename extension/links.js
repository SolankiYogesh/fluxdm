const $ = (id) => document.getElementById(id);
const id = new URLSearchParams(location.search).get("id");
const key = `picker:${id}`;
const data = (await chrome.storage.session.get(key))[key];

const el = (tag, text, className) => {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
};

let activeType = "Files";
// A hand-made selection starts fully ticked; a whole page starts with files only.
const rows = (data?.links ?? []).map((link) => {
  const box = el("input");
  box.type = "checkbox";
  box.checked = data.selectionOnly || !["Other", "Images"].includes(link.type);
  if (data.selectionOnly) activeType = "All";
  const tr = el("tr");
  const pick = el("td", undefined, "pick");
  pick.append(box);
  const address = el("td", link.url);
  address.title = link.url;
  tr.append(pick, el("td", link.type), el("td", link.text), address);
  box.addEventListener("change", count);
  $("rows").append(tr);
  return { link, box, tr };
});

const matchesType = (type) =>
  activeType === "All" || (activeType === "Files" ? type !== "Other" : type === activeType);

function visible() {
  return rows.filter((r) => !r.tr.hidden);
}

// Only ticked rows that are on screen are downloaded, so nothing hidden slips in.
function chosen() {
  return visible().filter((r) => r.box.checked);
}

function count() {
  const n = chosen().length;
  $("go").textContent = `Download ${n} file(s)`;
  $("go").disabled = n === 0;
  $("status").textContent = `${visible().length} shown of ${rows.length} links`;
}

function filter() {
  const terms = $("search").value.toLowerCase().split(/\s+/).filter(Boolean);
  for (const r of rows) {
    const hay = `${r.link.url} ${r.link.text}`.toLowerCase();
    r.tr.hidden = !matchesType(r.link.type) || !terms.every((t) => hay.includes(t));
  }
  for (const b of $("types").children) b.disabled = b.dataset.type === activeType;
  count();
}

for (const type of ["Files", "All", ...new Set(rows.map((r) => r.link.type))]) {
  const button = el("button", type);
  button.type = "button";
  button.dataset.type = type;
  button.addEventListener("click", () => {
    activeType = type;
    // Choosing one type selects exactly what is then shown.
    if (type !== "All" && type !== "Files") rows.forEach((r) => (r.box.checked = r.link.type === type));
    filter();
  });
  $("types").append(button);
}

$("search").addEventListener("input", filter);
$("all").addEventListener("click", () => {
  visible().forEach((r) => (r.box.checked = true));
  count();
});
$("none").addEventListener("click", () => {
  visible().forEach((r) => (r.box.checked = false));
  count();
});
$("go").addEventListener("click", async () => {
  $("go").disabled = true;
  $("status").textContent = "Sending to FluxDM…";
  const reply = await chrome.runtime.sendMessage({ type: "download-picked", id, urls: chosen().map((r) => r.link.url) });
  if (reply?.ok) {
    $("status").textContent = `Sent ${reply.accepted} download(s).`;
    await chrome.storage.session.remove(key);
    setTimeout(() => window.close(), 1200);
  } else {
    $("status").textContent = `Failed: ${reply?.error ?? "no reply"}`;
    $("go").disabled = false;
  }
});

filter();
if (!data) $("status").textContent = "This list has expired. Open it again from the page.";
