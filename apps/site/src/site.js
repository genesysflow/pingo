const root = document.documentElement;
const themeButton = document.querySelector(".theme-toggle");
const productImage = document.querySelector("#product-image");
let preview = "chat";

function updateTheme() {
  const dark = root.dataset.theme === "dark";
  themeButton.setAttribute(
    "aria-label",
    `Switch to ${dark ? "light" : "dark"} theme`,
  );
  themeButton.title = themeButton.getAttribute("aria-label");
  if (productImage) {
    productImage.src = `${productImage.dataset.assetBase}${preview}-${dark ? "dark" : "light"}.png`;
    productImage.alt =
      preview === "chat"
        ? "Pingo example workspace with project rooms, a threaded conversation, reactions and a room call."
        : "Pingo screen-sharing example with a shared design, call participants, pointers and call controls.";
  }
}

themeButton.addEventListener("click", () => {
  root.dataset.theme = root.dataset.theme === "dark" ? "light" : "dark";
  try {
    localStorage.setItem("pingo-site-theme", root.dataset.theme);
  } catch {}
  updateTheme();
});
for (const button of document.querySelectorAll("[data-preview]")) {
  button.addEventListener("click", () => {
    preview = button.dataset.preview;
    for (const tab of document.querySelectorAll("[data-preview]"))
      tab.setAttribute("aria-pressed", String(tab === button));
    updateTheme();
  });
}
updateTheme();

// Native details keeps every documentation link reachable without JavaScript.
const menu = document.querySelector(".docs-menu");
if (menu) {
  const mobile = matchMedia("(max-width: 800px)");
  const update = () => {
    menu.open = !mobile.matches;
  };
  mobile.addEventListener("change", update);
  update();
}

for (const pre of document.querySelectorAll(".prose pre")) {
  const code = pre.querySelector("code");
  if (!code || !navigator.clipboard?.writeText) continue;
  const button = document.createElement("button");
  button.className = "copy-code";
  button.type = "button";
  button.textContent = "Copy";
  button.setAttribute("aria-label", "Copy code to clipboard");
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(code.textContent);
      button.textContent = "Copied";
    } catch {
      button.textContent = "Copy failed";
    }
    setTimeout(() => {
      button.textContent = "Copy";
    }, 2000);
  });
  pre.append(button);
}
