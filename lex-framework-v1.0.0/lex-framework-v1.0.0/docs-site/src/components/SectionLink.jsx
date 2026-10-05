import React from "react";

// HashRouter owns the URL fragment. Section jumps must not replace its route.
export function SectionLink({ id, children, ...props }) {
  return (
    <a
      {...props}
      href={`#${id}`}
      onClick={(event) => {
        event.preventDefault();
        const section = document.getElementById(id);
        if (!section) return;
        section.scrollIntoView({ behavior: "smooth", block: "start" });
        section.setAttribute("tabindex", "-1");
        section.focus({ preventScroll: true });
      }}
    >
      {children}
    </a>
  );
}

export function TableOfContents({ sections }) {
  if (!sections.length) return null;
  return (
    <aside className="toc desktop-only" aria-label="On this page">
      <div className="toc-title">On this page</div>
      {sections.map(({ id, title }) => (
        <SectionLink key={id} id={id}>{title}</SectionLink>
      ))}
    </aside>
  );
}
