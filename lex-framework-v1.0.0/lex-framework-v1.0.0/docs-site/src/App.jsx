import React, { useEffect, useMemo, useState } from "react";
import { Link, NavLink, Route, Routes, useLocation, useNavigate, useParams } from "react-router-dom";
import { AnimatePresence, motion } from "framer-motion";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  ArrowRight, BookOpen, Box, Check, ChevronDown, Clipboard, Code2, Command,
  Database, ExternalLink, FileCode2, GitFork, KeyRound, Layers3, Menu, Moon,
  PackageCheck, Search, ShieldCheck, Sparkles, Sun, TerminalSquare, X, Zap
} from "lucide-react";
import { DOC_VERSION, docsNav } from "./docsManifest";

const rawDocs = import.meta.glob("./content/v1.0.0/*.md", { query: "?raw", import: "default", eager: true });
const docsBySlug = Object.fromEntries(Object.entries(rawDocs).map(([path, value]) => [path.split("/").pop().replace(/\.md$/, ""), value]));

function stripMarkdown(s="") { return s.replace(/```[\s\S]*?```/g," ").replace(/`([^`]+)`/g,"$1").replace(/[#>*_\-[\]()]/g," ").replace(/\s+/g," ").trim(); }
const flatDocs = docsNav.flatMap(group => group.items.map(([title, slug]) => ({ title, slug, group: group.group, text: stripMarkdown(docsBySlug[slug] || "") })));

function Logo() {
  return <Link className="brand" to="/" aria-label="Lex Framework home"><span className="brand-mark">LEX</span><span><b>Lex</b><small>Framework</small></span></Link>;
}

function ThemeButton({theme,setTheme}) {
  return <button className="icon-btn" onClick={()=>setTheme(theme === "dark" ? "light" : "dark")} aria-label="Toggle theme">{theme === "dark" ? <Sun size={18}/> : <Moon size={18}/>}</button>;
}

function Header({theme,setTheme,onMenu,onSearch}) {
  return <header className="topbar"><div className="topbar-inner"><button className="icon-btn mobile-only" onClick={onMenu} aria-label="Open navigation"><Menu size={19}/></button><Logo/><nav className="topnav desktop-only"><NavLink to="/docs/quick-start">Docs</NavLink><NavLink to="/docs/cli">CLI</NavLink><NavLink to="/docs/permissions">Permissions</NavLink><NavLink to="/docs/production-checklist">Production</NavLink></nav><div className="header-actions"><button className="search-trigger" onClick={onSearch}><Search size={16}/><span>Search docs</span><kbd>⌘K</kbd></button><span className="version-pill">v{DOC_VERSION}</span><ThemeButton theme={theme} setTheme={setTheme}/></div></div></header>;
}

function Sidebar({open,onClose}) {
 const content = <aside className="sidebar"><div className="sidebar-head"><Logo/><button className="icon-btn mobile-only" onClick={onClose}><X size={18}/></button></div><div className="sidebar-version"><span>Documentation</span><strong>v{DOC_VERSION}</strong></div><nav>{docsNav.map(group => <div className="nav-group" key={group.group}><div className="nav-group-title">{group.group}</div>{group.items.map(([title,slug]) => <NavLink key={slug} onClick={onClose} to={`/docs/${slug}`} className={({isActive})=>isActive?"active":""}>{title}</NavLink>)}</div>)}</nav></aside>;
 return <><div className="desktop-sidebar desktop-only">{content}</div><AnimatePresence>{open && <><motion.div className="drawer-overlay mobile-only" initial={{opacity:0}} animate={{opacity:1}} exit={{opacity:0}} onClick={onClose}/><motion.div className="drawer mobile-only" initial={{x:-340}} animate={{x:0}} exit={{x:-340}} transition={{type:"spring",stiffness:320,damping:32}}>{content}</motion.div></>}</AnimatePresence></>;
}

function CodeBlock({children,className}) {
 const [copied,setCopied]=useState(false); const text=String(children).replace(/\n$/,""); const lang=(className||"").replace("language-","") || "text";
 return <div className="code-wrap"><div className="code-head"><span>{lang}</span><button onClick={async()=>{await navigator.clipboard?.writeText(text);setCopied(true);setTimeout(()=>setCopied(false),1200)}}>{copied?<Check size={14}/>:<Clipboard size={14}/>} {copied?"Copied":"Copy"}</button></div><pre><code className={className}>{text}</code></pre></div>;
}

function slugify(children) { const text=React.Children.toArray(children).join(" "); return text.toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-|-$/g,""); }
const docLinkAliases = {
  "installation.md": "installation",
  "quickstart.md": "quick-start",
  "architecture.md": "architecture",
  "databases.md": "database",
  "models-and-queries.md": "models-querysets",
  "transactions.md": "transactions",
  "migrations.md": "migrations",
  "authentication.md": "authentication",
  "security.md": "security",
  "rbac.md": "permissions",
  "rest-api.md": "rest-framework",
  "application-boundaries.md": "media",
  "configuration.md": "settings",
  "cli.md": "cli",
  "deployment.md": "operations",
  "release.md": "packaging",
  "TEST_MATRIX.md": "testing",
  "REPORT.md": "release-notes",
  "CONTRIBUTING.md": "testing",
  "README.md": "introduction",
  "first_api.rs": "rest-framework",
  "first_model.rs": "quick-start",
  "tenant_http.rs": "testing",

  "QUICKSTART.md": "quick-start",
  "VIEWS_AND_URLS.md": "views-and-urls",
  "AUTHENTICATION.md": "authentication",
  "PERMISSIONS.md": "permissions",
  "MIDDLEWARE.md": "middleware-security",
  "SETTINGS.md": "settings",
  "MEDIA.md": "media",
  "RECIPES.md": "recipes",
  "website/README.md": "introduction",
};
function docHref(href="") {
  const clean = href.replace(/^\.\//, "");
  const target = docLinkAliases[clean] || docLinkAliases[clean.split("/").pop()];
  if (target) return `#/docs/${target}`;
  if (clean.includes("integration/") || clean.includes("crate/") || clean.endsWith(".rs")) return "#/docs/testing";
  return href;
}

const markdownComponents = {
 h1: ({children}) => <h1 id={slugify(children)}>{children}</h1>, h2: ({children}) => <h2 id={slugify(children)}>{children}</h2>, h3: ({children}) => <h3 id={slugify(children)}>{children}</h3>,
 a: ({href,children}) => { const external=href?.startsWith("http"); const resolved=external?href:docHref(href); return <a href={resolved} target={external?"_blank":undefined} rel="noreferrer">{children}{external?<ExternalLink size={12}/>:null}</a>; },
 pre: ({children}) => <>{children}</>,
 code: ({className,children,...props}) => { const block = Boolean(className) || String(children).includes("\n"); return block ? <CodeBlock className={className}>{children}</CodeBlock> : <code className="inline-code" {...props}>{children}</code>; },
 table: ({children}) => <div className="table-scroll"><table>{children}</table></div>,
 blockquote: ({children}) => <blockquote>{children}</blockquote>
};

function extractToc(md) { return md.split("\n").filter(x=>/^##\s/.test(x)).slice(0,12).map(x=>{const title=x.replace(/^##\s+/,"").replace(/[`*_]/g,""); return {title,id:slugify(title)}}); }

function DocPage() {
 const {slug="quick-start"}=useParams(); const md=docsBySlug[slug]; const page=flatDocs.find(x=>x.slug===slug); const toc=useMemo(()=>extractToc(md||""),[md]);
 if(!md) return <NotFound/>;
 return <motion.main className="doc-shell" key={slug} initial={{opacity:0,y:8}} animate={{opacity:1,y:0}} transition={{duration:.25}}><article className="doc-article"><div className="eyebrow">{page?.group} · v{DOC_VERSION}</div><ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{md}</ReactMarkdown><DocPager slug={slug}/></article><aside className="toc desktop-only"><div className="toc-title">On this page</div>{toc.map(x=><a href={`#${x.id}`} key={x.id}>{x.title}</a>)}</aside></motion.main>;
}
function DocPager({slug}) { const idx=flatDocs.findIndex(x=>x.slug===slug); const prev=flatDocs[idx-1], next=flatDocs[idx+1]; return <div className="doc-pager">{prev?<Link to={`/docs/${prev.slug}`}><small>Previous</small><strong>← {prev.title}</strong></Link>:<span/>}{next?<Link to={`/docs/${next.slug}`} className="next"><small>Next</small><strong>{next.title} →</strong></Link>:null}</div>; }

function SearchModal({open,onClose}) {
 const [q,setQ]=useState(""); const navigate=useNavigate(); useEffect(()=>{if(!open)setQ("")},[open]); useEffect(()=>{const f=e=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==="k"){e.preventDefault();open?onClose():null}};window.addEventListener("keydown",f);return()=>window.removeEventListener("keydown",f)},[open,onClose]);
 const results=useMemo(()=>{const t=q.trim().toLowerCase(); if(!t)return flatDocs.slice(0,8); return flatDocs.map(d=>({...d,score:(d.title.toLowerCase().includes(t)?5:0)+(d.group.toLowerCase().includes(t)?2:0)+(d.text.toLowerCase().includes(t)?1:0)})).filter(x=>x.score).sort((a,b)=>b.score-a.score).slice(0,12)},[q]);
 return <AnimatePresence>{open&&<motion.div className="search-overlay" initial={{opacity:0}} animate={{opacity:1}} exit={{opacity:0}} onMouseDown={onClose}><motion.div className="search-modal" initial={{scale:.97,y:-8}} animate={{scale:1,y:0}} exit={{scale:.97,y:-8}} onMouseDown={e=>e.stopPropagation()}><div className="search-input"><Search size={19}/><input autoFocus value={q} onChange={e=>setQ(e.target.value)} placeholder="Search Lex v1.0.0 documentation"/><button onClick={onClose}><X size={17}/></button></div><div className="search-results">{results.map(r=><button key={r.slug} onClick={()=>{navigate(`/docs/${r.slug}`);onClose()}}><div><strong>{r.title}</strong><span>{r.group}</span></div><ArrowRight size={16}/></button>)}{results.length===0&&<div className="empty-search">No documentation pages matched.</div>}</div></motion.div></motion.div>}</AnimatePresence>;
}

const features=[
 [Database,"PostgreSQL + SQLite","PostgreSQL-first power with an automatic SQLite development path."],
 [Layers3,"Typed ORM","Derived models, metadata, QuerySets, relations, transactions and migrations."],
 [KeyRound,"Framework auth","Sessions, signed cookies and bearer tokens configured instead of reimplemented."],
 [ShieldCheck,"Declarative permissions","Protected-by-default views with public, scope, tenant, role and owner rules."],
 [Zap,"REST runtime","Views, routing, middleware, idempotency, throttling, auditing and WebSockets."],
 [Box,"Media pipeline","Filesystem/memory storage, S3-compatible configuration and ClamAV scanning."],
 [Command,"Developer CLI","Create projects and crates, validate, migrate, verify and run from `lex`."],
 [PackageCheck,"Production controls","Security settings, operational checks, migration discipline and verification."],
];

function Home() {
 return <main className="home"><section className="hero"><motion.div className="hero-copy" initial={{opacity:0,y:20}} animate={{opacity:1,y:0}} transition={{duration:.55}}><div className="hero-badge"><Sparkles size={15}/> Lex Framework <b>v{DOC_VERSION}</b></div><h1>Build large Rust backends<br/><span>without rebuilding the framework.</span></h1><p>A PostgreSQL-first, SQLite-friendly application framework for typed models, APIs, authentication, permissions, media, security and production operations.</p><div className="hero-actions"><Link className="button primary" to="/docs/quick-start">Start building <ArrowRight size={17}/></Link><Link className="button ghost" to="/docs/full-developer-guide"><BookOpen size={17}/> Developer guide</Link></div><div className="hero-command"><TerminalSquare size={18}/><code>lex create crate accounts --url /api/v1/accounts</code></div></motion.div><motion.div className="hero-panel" initial={{opacity:0,scale:.96}} animate={{opacity:1,scale:1}} transition={{duration:.65,delay:.1}}><div className="terminal"><div className="terminal-bar"><i/><i/><i/><span>lex</span></div><pre><span className="dim">$</span> lex create settings --project-name marketplace\n<span className="ok">✓</span> Created project settings\n\n<span className="dim">$</span> lex create crate products --url /api/v1/products\n<span className="ok">✓</span> Generated model, view and URL structure\n<span className="ok">✓</span> Registered application routes\n\n<span className="dim">$</span> lex start server\n<span className="ok">✓</span> SQLite ready\n<span className="ok">✓</span> Security middleware ready\n<span className="ok">✓</span> Listening on 127.0.0.1:8080</pre></div></motion.div></section><section className="stats"><div><strong>1</strong><span>framework facade</span></div><div><strong>2</strong><span>database backends</span></div><div><strong>3</strong><span>first-class auth modes</span></div><div><strong>v1</strong><span>release candidate</span></div></section><section className="home-section"><div className="section-kicker">One coherent framework</div><h2>Common backend infrastructure is already part of the platform.</h2><div className="feature-grid">{features.map(([Icon,title,copy],i)=><motion.div className="feature-card" key={title} initial={{opacity:0,y:18}} whileInView={{opacity:1,y:0}} viewport={{once:true,margin:"-50px"}} transition={{delay:(i%4)*.06}}><Icon size={22}/><h3>{title}</h3><p>{copy}</p></motion.div>)}</div></section><section className="workflow home-section"><div><div className="section-kicker">Developer flow</div><h2>Settings → models → views → permissions → logic.</h2><p>The framework owns the plumbing. Application crates own domain behavior.</p><Link className="text-link" to="/docs/views-and-urls">See the view model <ArrowRight size={16}/></Link></div><div className="flow-list">{["Choose settings","Create application crate","Define model","Write APIView","Declare permissions","Run with lex"].map((x,i)=><div key={x}><span>{String(i+1).padStart(2,"0")}</span><strong>{x}</strong></div>)}</div></section><section className="cta"><div><span className="section-kicker">Version 1.0.0</span><h2>A release candidate with recorded validation gates.</h2><p>Review the executed tests and remaining release blockers before deployment.</p></div><Link className="button primary" to="/docs/versioning">Versioning policy <ArrowRight size={17}/></Link></section></main>;
}

function NotFound(){return <main className="notfound"><FileCode2 size={38}/><h1>Page not found</h1><p>The requested documentation page does not exist in v{DOC_VERSION}.</p><Link className="button primary" to="/docs/quick-start">Open quick start</Link></main>}

function App(){
 const [theme,setTheme]=useState(()=>localStorage.getItem("lex-theme")||"dark"); const [menu,setMenu]=useState(false); const [search,setSearch]=useState(false); const location=useLocation();
 useEffect(()=>{document.documentElement.dataset.theme=theme;localStorage.setItem("lex-theme",theme)},[theme]);
 useEffect(()=>{window.scrollTo({top:0,behavior:"instant"});setMenu(false)},[location.pathname]);
 useEffect(()=>{const fn=e=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==="k"){e.preventDefault();setSearch(true)} if(e.key==="Escape")setSearch(false)};window.addEventListener("keydown",fn);return()=>window.removeEventListener("keydown",fn)},[]);
 const isDocs=location.pathname.startsWith("/docs/");
 return <div className="app"><Header theme={theme} setTheme={setTheme} onMenu={()=>setMenu(true)} onSearch={()=>setSearch(true)}/>{isDocs&&<Sidebar open={menu} onClose={()=>setMenu(false)}/>}<div className={isDocs?"content with-sidebar":"content"}><Routes><Route path="/" element={<Home/>}/><Route path="/docs/:slug" element={<DocPage/>}/><Route path="*" element={<NotFound/>}/></Routes></div><SearchModal open={search} onClose={()=>setSearch(false)}/><footer><div><Logo/><p>Lex Framework documentation · Version {DOC_VERSION}</p></div><div><Link to="/docs/security">Security</Link><Link to="/docs/operations">Operations</Link><Link to="/docs/versioning">Versioning</Link></div></footer></div>;
}

export default App;
