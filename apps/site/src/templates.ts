import { escape, pages } from "./pages";

export function templates(base: string, repository: string) {
  const url = (path = "") => `${base}${path}`;
  const arrow = '<span aria-hidden="true">↗</span>';
  const sourceLink = repository
    ? `<a href="${escape(repository)}">GitHub ${arrow}</a>`
    : "";
  const brand = `<a class="brand" href="${url()}" aria-label="Pingo home"><img src="${url("assets/pingo.svg")}" width="32" height="32" alt="">pingo<span class="brand-dot">.</span></a>`;

  function layout(
    title: string,
    description: string,
    body: string,
    docs = false,
  ) {
    return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="description" content="${escape(description)}">
  <meta name="theme-color" content="#f8f7f4">
  <meta property="og:title" content="${escape(title)}">
  <meta property="og:description" content="${escape(description)}">
  <meta property="og:type" content="website">
  <title>${escape(title)}</title>
  <link rel="icon" type="image/svg+xml" href="${url("assets/pingo.svg")}">
  <script>try{const t=localStorage.getItem('pingo-site-theme');if(t==='dark')document.documentElement.dataset.theme=t}catch{}</script>
  <link rel="stylesheet" href="${url("assets/style.css")}">
  <script src="${url("assets/site.js")}" defer></script>
</head>
<body${docs ? ' class="docs-page"' : ""}>
  <a class="skip-link" href="#main">Skip to content</a>
  <header class="site-header"><div class="header-inner">
    ${brand}
    <nav class="header-nav" aria-label="Main navigation">
      <a href="${url()}#inside">Product</a>
      <a href="${url("docs/overview/")}"${docs ? ' aria-current="true"' : ""}>Docs</a>
      ${sourceLink}
    </nav>
    <div class="header-actions"><button class="theme-toggle" type="button" aria-label="Switch to dark theme" title="Switch theme"><span aria-hidden="true">◐</span></button><a class="button small" href="${url("docs/self-hosting/")}">Get started <span aria-hidden="true">↗</span></a></div>
  </div></header>
  ${body}
  <footer class="site-footer"><div class="footer-inner"><div>${brand}<p>A place for your people. On your terms.</p></div><nav aria-label="Footer navigation"><a href="${url("docs/self-hosting/")}">Self-hosting</a><a href="${url("docs/architecture/")}">Architecture</a>${sourceLink}</nav><span class="footer-note">Built with Rust & Svelte.</span></div></footer>
</body>
</html>`;
  }

  function home() {
    return layout(
      "Pingo — Your people. Your own space.",
      "Chat, voice and screen sharing for gaming groups, friends, communities and teams. Meet Pingo and make a space on a server you run yourself.",
      `
<main id="main">
  <section class="hero wrap" aria-labelledby="hero-title">
    <div class="eyebrow"><span class="status-dot"></span> SELF-HOSTED CHAT & VOICE <span class="hero-status">IN DEVELOPMENT</span></div>
    <h1 id="hero-title">Your people.<br>Your own <span>space.</span></h1>
    <div class="hero-bottom"><p>For game nights, shared projects and hanging out.<br class="desktop-break"> Your friends, community or team. Your own server.</p><div class="hero-buttons"><a class="button" href="${url("docs/self-hosting/")}">Host your Pingo <span aria-hidden="true">↗</span></a><a class="text-link" href="${url("docs/overview/")}">Read the docs <span aria-hidden="true">→</span></a></div></div>
  </section>
  <section class="product-preview wrap" aria-label="Explore the interface">
    <div class="preview-toolbar"><div class="preview-tabs" role="group" aria-label="Product screenshots"><button type="button" data-preview="chat" aria-pressed="true">01 <span>The conversation</span></button><button type="button" data-preview="share" aria-pressed="false">02 <span>The shared screen</span></button></div><span class="preview-caption">ONE EXAMPLE SPACE <span aria-hidden="true">↙</span></span></div>
    <div class="preview-window"><img id="product-image" src="${url("assets/chat-light.png")}" data-asset-base="${url("assets/")}" width="1440" height="900" fetchpriority="high" alt="Pingo example workspace with project rooms, a threaded conversation, reactions and a room call."></div>
    <div class="preview-foot"><span><span class="status-dot"></span> Chat + voice, in the same room.</span><span>WEB & DESKTOP</span></div>
    <noscript><p>Also see <a href="${url("assets/share-light.png")}">the screen-sharing interface</a>.</p></noscript>
  </section>
  <section class="features wrap" id="inside" aria-labelledby="features-title">
    <div class="section-heading"><p class="eyebrow">PLAY. HANG OUT. MAKE THINGS.</p><h2 id="features-title">From “who’s around?”<br>to “let me show you.”</h2></div>
    <div class="feature-grid">
      <article><span class="feature-number">01 / CHAT</span><h3>Keep the thread.</h3><p>Rooms for your games, shared interests and projects. Plan the next session, trade ideas in threads, or catch up in DMs. Your mentions and replies wait in one inbox.</p><a class="text-link" href="${url("docs/overview/")}#a-space-for-your-people">Find your way around <span aria-hidden="true">→</span></a></article>
      <article><span class="feature-number">02 / VOICE</span><h3>Get the group together.</h3><p>Join voice for a co-op session, a community hangout or a team catch-up. Every room has a call. Full-band Opus and noise suppression keep the conversation clear.</p><a class="text-link" href="${url("docs/overview/")}#voice-and-studio">About the audio <span aria-hidden="true">→</span></a></article>
      <article><span class="feature-number">03 / SHARE</span><h3>Share the moment.</h3><p>Show a game, walk through a build or review a design together. Choose Motion for movement or Text for detail, with pointers and drawing on the shared screen.</p><a class="text-link" href="${url("docs/web/")}#calls">How sharing works <span aria-hidden="true">→</span></a></article>
    </div>
  </section>
  <section class="audio-section" aria-labelledby="audio-title"><div class="audio-inner wrap">
    <div class="audio-copy"><p class="eyebrow">ROOM FOR A BETTER CONVERSATION</p><h2 id="audio-title">Good audio.<br>Fewer distractions.</h2><p>Keep keyboard clatter out of the call, or turn processing off for music in stereo Studio mode. The browser and native desktop engine use the same DeepFilterNet3 model.</p><a class="text-link" href="${url("docs/denoise/")}">Inside noise suppression <span aria-hidden="true">↗</span></a></div>
    <div class="audio-spec" aria-label="Audio features"><div class="spec-header"><span class="status-dot"></span> THE AUDIO TOOLKIT <span aria-hidden="true">↗</span></div><dl><div><dt>Full-band Opus</dt><dd>48 <span>kHz</span></dd></div><div><dt>Noise suppression</dt><dd class="spec-text">Off / Gentle / Strong</dd></div><div><dt>Studio mode</dt><dd class="spec-text">Stereo. Processing off.</dd></div><div><dt>Before you join</dt><dd class="spec-text">A real microphone check.</dd></div></dl><p>Actual call quality depends on your devices and network.</p></div>
  </div></section>
  <section class="hosting-section wrap" aria-labelledby="hosting-title"><div><p class="eyebrow">YOUR SERVER. YOUR CALL.</p><h2 id="hosting-title">Make yourself<br>at home.</h2><p>Run the setup script or use Docker Compose directly. No Git or build tools needed. An embedded database. A data directory you can back up. Choose who can join, and keep your group’s space under your control.</p><a class="button" href="${url("docs/self-hosting/")}">Set up your server <span aria-hidden="true">↗</span></a></div><ol class="hosting-steps"><li><span>01</span><div><h3>Bring a server & a domain.</h3><p>Linux, Docker Compose, HTTPS and a reachable UDP port for calls.</p></div></li><li><span>02</span><div><h3>Configure. Start. Invite.</h3><p>Run the setup script, enter your domain and public IP, and let Docker Compose start Pingo.</p></div></li><li><span>03</span><div><h3>Keep it yours.</h3><p>Use invites and email allowlists. Back up your data and upgrade on your schedule.</p></div></li></ol></section>
  <aside class="development-note wrap"><span class="status-dot"></span><p><strong>A work in progress, ready to explore.</strong> Pingo is in active development. Calls prefer direct UDP and can fall back to an optional TURN relay. <a href="${url("docs/overview/")}#current-boundaries">Read the current boundaries <span aria-hidden="true">↗</span></a></p></aside>
</main>`,
    );
  }

  function doc(
    page: (typeof pages)[number],
    content: string,
    toc: { id: string; text: string }[],
    index: number,
  ) {
    const groups = [...new Set(pages.map((p) => p.group))];
    const sidebar = groups
      .map(
        (group) =>
          `<div class="sidebar-group"><p>${group}</p>${pages
            .filter((p) => p.group === group)
            .map(
              (p) =>
                `<a href="${url(`docs/${p.slug}/`)}"${p.slug === page.slug ? ' aria-current="page"' : ""}>${p.title}</a>`,
            )
            .join("")}</div>`,
      )
      .join("");
    const previous = pages[index - 1];
    const next = pages[index + 1];
    const download = `<aside class="setup-download"><strong>Start with the setup script</strong><p>Enter your domain and public IP. The script generates your configuration and starts Pingo with Docker.</p><pre><code data-install-command data-install-path="${url("install.sh")}">curl -fsSL https://raw.githubusercontent.com/genesysflow/pingo/main/deploy/install.sh | bash</code></pre><a class="text-link" href="${url("install.sh")}">View setup script <span aria-hidden="true">↗</span></a><a class="text-link" href="#docker-compose">Use Docker Compose instead <span aria-hidden="true">→</span></a></aside>`;
    const docContent =
      page.slug === "self-hosting"
        ? content.replace("</h1>", `</h1>${download}`)
        : content;
    return layout(
      `${page.title} — Pingo docs`,
      `Pingo documentation: ${page.title}. Setup instructions and technical reference for self-hosted chat, voice and screen sharing.`,
      `
<div class="docs-shell wrap">
  <details class="docs-menu" open><summary>Documentation <span aria-hidden="true">⌄</span></summary><nav aria-label="Documentation">${sidebar}</nav></details>
  <main class="doc-main" id="main"><div class="doc-breadcrumb"><a href="${url("docs/overview/")}">Docs</a><span aria-hidden="true">/</span><span>${page.group}</span></div><article class="prose">${docContent}</article><div class="doc-source"><a href="${url(`sources/${page.source}`)}">Markdown source <span aria-hidden="true">↗</span></a>${repository ? `<a href="${escape(repository)}/blob/main/${page.source}">View on GitHub <span aria-hidden="true">↗</span></a>` : ""}</div><nav class="doc-pagination" aria-label="Previous and next pages">${previous ? `<a href="${url(`docs/${previous.slug}/`)}"><small>← Previous</small>${previous.title}</a>` : "<span></span>"}${next ? `<a href="${url(`docs/${next.slug}/`)}"><small>Next →</small>${next.title}</a>` : ""}</nav></main>
  <aside class="doc-toc"><nav aria-label="On this page"><p>ON THIS PAGE</p>${toc.map((h) => `<a href="#${escape(h.id)}">${escape(h.text)}</a>`).join("")}</nav></aside>
</div>`,
      true,
    );
  }

  function notFound() {
    return layout(
      "Page not found — Pingo",
      "This page could not be found.",
      `<main class="not-found wrap" id="main"><p class="eyebrow">404 / PAGE NOT FOUND</p><h1>A room that<br>isn’t here.</h1><p>The link may have moved. Let’s get you back.</p><a class="button" href="${url()}">Back to Pingo <span aria-hidden="true">→</span></a><a class="text-link" href="${url("docs/overview/")}">Browse the docs</a></main>`,
    );
  }
  return { home, doc, notFound };
}
