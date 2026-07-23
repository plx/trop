/* trop site — UI kit screens.
 * Faithful recreation of the trop marketing/docs landing page, composed from
 * the design-system components. Exposes sections on window for index.html.
 */
const { Button, Badge, Eyebrow, ThemeToggle, CommandCard, FeatureCard, DocCard, ScopePanel } =
  window.TropDesignSystem_9d5100;

const MARK = "../../assets/tool-mark.svg";

const ghIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"/>
  </svg>
);

const SHELL = { width: "min(1120px, calc(100% - 2rem))", marginInline: "auto" };

function SiteHeader() {
  const [open, setOpen] = React.useState(false);
  const [hover, setHover] = React.useState(null);
  const links = [["Overview", "#features"], ["Usage", "#docs"], ["Scope", "#scope"], ["Install", "#install"]];
  return (
    <header style={{ position: "sticky", top: 0, zIndex: 10, borderBottom: "1px solid color-mix(in srgb, var(--tool-line) 78%, transparent)", background: "color-mix(in srgb, var(--surface-page) 94%, transparent)", backdropFilter: "blur(12px)" }}>
      <nav style={{ ...SHELL, display: "flex", alignItems: "center", justifyContent: "space-between", gap: "1rem", minHeight: 72 }}>
        <a href="#top" style={{ display: "flex", alignItems: "center", gap: "0.7rem", fontFamily: "var(--font-display)", fontWeight: 800, fontSize: "1.25rem", color: "var(--tool-ink)", textDecoration: "none" }}>
          <img src={MARK} alt="" width="36" height="36" />trop
        </a>
        <div style={{ display: "flex", alignItems: "center", gap: "0.35rem" }} className="nav-links">
          {links.map(([label, href]) => (
            <a key={href} href={href}
              onMouseEnter={() => setHover(href)} onMouseLeave={() => setHover(null)}
              style={{ padding: "0.55rem 0.7rem", borderRadius: "var(--tool-radius)", color: hover === href ? "var(--tool-ink)" : "var(--tool-muted)", fontWeight: 700, textDecoration: "none", background: hover === href ? "var(--accent-tint-10)" : "transparent" }}>
              {label}
            </a>
          ))}
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
          <ThemeToggle />
          <Button size="sm" variant="ghost" icon={ghIcon} href="https://github.com/prb/trop">GitHub</Button>
          <Button size="sm" variant="primary" href="#install">Install</Button>
        </div>
      </nav>
    </header>
  );
}

function Hero() {
  return (
    <section id="top" style={{ position: "relative", overflow: "hidden", borderBottom: "1px solid var(--tool-line)" }}>
      <div className="hero-art" aria-hidden="true" style={{ position: "absolute", inset: 0 }} />
      <div style={{ position: "relative", ...SHELL, display: "grid", gridTemplateColumns: "minmax(0, 1fr) minmax(320px, 0.74fr)", gap: "clamp(2rem, 6vw, 5rem)", alignItems: "center", minHeight: 640, paddingBlock: "clamp(4rem, 10vw, 6rem)" }}>
        <div style={{ minWidth: 0 }}>
          <Eyebrow>Port reservations for agentic coding</Eyebrow>
          <h1 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 800, fontSize: "clamp(4.75rem, 12vw, 7.5rem)", lineHeight: 0.9, letterSpacing: "-0.02em", color: "var(--tool-ink)" }}>trop</h1>
          <p style={{ maxWidth: "16ch", margin: "1rem 0 0", fontSize: "clamp(1.5rem, 4vw, 2.6rem)", fontWeight: 700, lineHeight: 1.08, color: "var(--tool-ink)" }}>Stable, directory-aware localhost ports.</p>
          <p style={{ maxWidth: "56ch", margin: "1.1rem 0 0", color: "var(--tool-muted)", fontSize: "1.08rem" }}>
            A small Rust CLI that replaces hardcoded port numbers with sticky, idempotent reservations — so concurrent worktrees and local agents never collide.
          </p>
          <div style={{ display: "flex", flexWrap: "wrap", gap: "0.55rem", marginTop: "1.5rem" }}>
            <Badge>Idempotent</Badge>
            <Badge>Directory-aware</Badge>
            <Badge>Cross-process safe</Badge>
            <Badge>SQLite-backed</Badge>
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: "0.8rem", marginTop: "2rem" }}>
            <Button variant="primary" href="#install">Install trop</Button>
            <Button variant="secondary" icon={ghIcon} href="https://github.com/prb/trop">Star on GitHub</Button>
          </div>
        </div>
        <CommandCard title="Reserve a port" meta="bash"
          code={'# reserve, then run your dev server\nPORT=$(trop reserve)\nnpm run dev -- --port "$PORT"\n\n# same directory → same port, every time'}
          copyText={'PORT=$(trop reserve)'} />
      </div>
    </section>
  );
}

function Section({ id, muted, children }) {
  return (
    <section id={id} style={{ paddingBlock: "clamp(4rem, 8vw, 6rem)", ...(muted ? { borderBlock: "1px solid var(--tool-line)", background: "color-mix(in srgb, var(--tool-panel) 45%, transparent)" } : {}) }}>
      <div style={SHELL}>{children}</div>
    </section>
  );
}

function SectionHeading({ eyebrow, title, children }) {
  return (
    <div style={{ maxWidth: 760, marginBottom: "1.5rem" }}>
      {eyebrow ? <Eyebrow>{eyebrow}</Eyebrow> : null}
      <h2 style={{ margin: 0, fontFamily: "var(--font-display)", fontWeight: 800, fontSize: "clamp(2rem, 5vw, 3.25rem)", lineHeight: 1.05, color: "var(--tool-ink)" }}>{title}</h2>
      {children ? <p style={{ maxWidth: "42rem", marginTop: "0.75rem", color: "var(--tool-muted)", fontSize: "1.08rem" }}>{children}</p> : null}
    </div>
  );
}

function Features() {
  const items = [
    ["01", "Idempotent", "Sticky reservations", "Reservations are keyed by directory and optional tag. Repeated calls return the same port — scripts stay stable across restarts."],
    ["02", "Lifecycle", "Directory-based cleanup", "Reservations are pruned once their worktree is removed. No teardown hooks wired into every dev script."],
    ["03", "Concurrency", "Cross-process safe", "Safe to invoke from many processes at once — several independent agents can reserve without a shared spreadsheet."],
    ["04", "Occupancy", "Conflict detection", "Verifies a port is unoccupied before reserving, and honors explicit exclusions of ports and ranges."],
    ["05", "Tags", "Multiple services", "Reserve distinct ports per service in one worktree with --tag web, --tag api, --tag db."],
    ["06", "Integration", "Drop-in replacement", "Swap a hardcoded PORT=4040 for PORT=$(trop reserve). That's the whole migration."],
  ];
  return (
    <Section id="features" muted>
      <SectionHeading eyebrow="Why trop" title="One job, done predictably">
        trop coordinates one user account on one machine. Everything below follows from keeping that scope narrow.
      </SectionHeading>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: "1rem" }} className="feature-grid">
        {items.map(([i, eb, t, body]) => (
          <FeatureCard key={i} index={i} eyebrow={eb} title={t}>{body}</FeatureCard>
        ))}
      </div>
    </Section>
  );
}

function Docs() {
  const docs = [
    ["Overview", "What trop reserves, why it exists, and where it fits."],
    ["Usage", "Basic commands and script patterns for local port reservations."],
    ["Configuration", "Port ranges, tags, exclusions, and cleanup behavior."],
    ["Scope", "What trop deliberately does and does not attempt to solve."],
  ];
  return (
    <Section id="docs">
      <SectionHeading eyebrow="Documentation" title="Guides">Short, practical pages — read in a couple of minutes.</SectionHeading>
      <div style={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: "1rem" }} className="docs-grid">
        {docs.map(([t, d]) => <DocCard key={t} title={t} linkLabel="Read guide">{d}</DocCard>)}
      </div>
    </Section>
  );
}

function Scope() {
  return (
    <Section id="scope" muted>
      <ScopePanel
        title="Deliberately narrow scope"
        intro="trop solves one problem: stable localhost port reservations for one user across local worktrees and processes."
        inScope={["Directory-aware reservations", "Optional tags for multiple services", "Concurrent local callers", "Local cleanup of stale reservations", "Avoiding occupied or excluded ports"]}
        nonGoals={["System-wide enforcement", "Multi-user coordination", "Container orchestration", "Network service discovery", "Replacing a process supervisor"]} />
    </Section>
  );
}

function Install() {
  return (
    <Section id="install">
      <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 0.72fr) minmax(320px, 0.28fr)", gap: "clamp(1.5rem, 5vw, 4rem)", alignItems: "end" }} className="install-grid">
        <div>
          <SectionHeading eyebrow="Install" title="Get trop from Cargo">
            A preview release: core functionality is implemented and heavily tested. Expect the occasional rough edge, and send feedback.
          </SectionHeading>
          <div style={{ display: "flex", gap: "0.8rem", flexWrap: "wrap" }}>
            <Button variant="secondary" icon={ghIcon} href="https://github.com/prb/trop">Read the source</Button>
            <Button variant="ghost" href="#docs">Browse docs</Button>
          </div>
        </div>
        <CommandCard compact meta="shell" code={'cargo install trop-cli'} copyText={'cargo install trop-cli'} />
      </div>
    </Section>
  );
}

function SiteFooter() {
  const [hover, setHover] = React.useState(null);
  const links = ["Overview", "Usage", "Configuration", "Scope", "GitHub"];
  return (
    <footer style={{ borderTop: "1px solid var(--tool-line)", paddingBlock: "2rem", color: "var(--tool-muted)" }}>
      <div style={{ ...SHELL, display: "flex", alignItems: "center", justifyContent: "space-between", gap: "1rem", flexWrap: "wrap" }}>
        <p style={{ margin: 0, display: "flex", alignItems: "center", gap: "0.6rem" }}>
          <img src={MARK} alt="" width="24" height="24" /> trop — dual-licensed Apache-2.0 / MIT.
        </p>
        <nav style={{ display: "flex", flexWrap: "wrap", justifyContent: "flex-end", gap: "0.35rem" }}>
          {links.map((l) => (
            <a key={l} href="#top"
              onMouseEnter={() => setHover(l)} onMouseLeave={() => setHover(null)}
              style={{ padding: "0.45rem 0.55rem", borderRadius: "var(--tool-radius)", color: hover === l ? "var(--tool-ink)" : "var(--tool-muted)", fontWeight: 700, textDecoration: "none", background: hover === l ? "var(--accent-tint-10)" : "transparent" }}>{l}</a>
          ))}
        </nav>
      </div>
    </footer>
  );
}

function TropSite() {
  return (
    <div style={{ background: "var(--surface-page)", minHeight: "100vh" }}>
      <SiteHeader />
      <Hero />
      <Features />
      <Docs />
      <Scope />
      <Install />
      <SiteFooter />
    </div>
  );
}

Object.assign(window, { TropSite });
