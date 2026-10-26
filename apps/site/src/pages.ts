export const pages = [
  {
    slug: "overview",
    title: "Meet Pingo",
    group: "Start here",
    source: "apps/site/src/content/overview.md",
  },
  {
    slug: "self-hosting",
    title: "Self-hosting",
    group: "Start here",
    source: "docs/SELF_HOSTING.md",
  },
  {
    slug: "architecture",
    title: "Architecture",
    group: "Build with Pingo",
    source: "apps/site/src/content/architecture.md",
  },
  {
    slug: "web",
    title: "Web client",
    group: "Build with Pingo",
    source: "apps/web/README.md",
  },
  {
    slug: "desktop",
    title: "Desktop client",
    group: "Build with Pingo",
    source: "apps/desktop/README.md",
  },
  {
    slug: "api",
    title: "Backend API",
    group: "Reference",
    source: "docs/API.md",
  },
  {
    slug: "sfu",
    title: "Media server",
    group: "Reference",
    source: "crates/pingo-sfu/README.md",
  },
  {
    slug: "media",
    title: "Native media",
    group: "Reference",
    source: "crates/pingo-media/README.md",
  },
  {
    slug: "denoise",
    title: "Noise suppression",
    group: "Reference",
    source: "crates/pingo-denoise/README.md",
  },
];

export const escape = (value: string) =>
  value.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
