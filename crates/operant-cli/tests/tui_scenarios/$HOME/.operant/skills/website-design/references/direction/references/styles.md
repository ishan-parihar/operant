# Style Reference

176 style entries (base styles plus best-fit variants). Source: ui-ux-pro-max styles.csv, normalized to the VARIANCE/MOTION/DENSITY dial system.

Column guide: WHEN TO USE gives dial ranges where the style is safe. Outside those ranges, pick another style or re-derive from the DIRECTION dials.

Reasoning: dial ranges encode product-type reasoning from `ui-ux-pro-max` (`styles.csv` + `ui-reasoning.csv` + `products.csv`). Minimalism allows high DENSITY (6-10) for dashboards because data outranks whitespace; Glassmorphism caps at low VARIANCE (1-4) because frosted depth breaks on dense layouts; Brutalism needs high VARIANCE/MOTION because raw asymmetry is the point. Best For / Do Not Use columns ARE the reasoning: a style's "Best For: B2B SaaS" plus "Do Not Use: playful entertainment" explains *why* its dial band is what it is. For per-row prose reasoning, see the source CSV `Reasoning` field.

## Base Styles

| # | Style | Keywords | Best For | Do Not Use | Dark Mode | VARIANCE | MOTION | DENSITY |
|---|-------|----------|----------|------------|-----------|----------|--------|---------|
| 1 | Minimalism & Swiss Style | Clean,  simple,  spacious,  functional,  white space,  high contrast,  geometric,  sans-serif,  grid-based,  essential | Enterprise apps,  dashboards,  documentation sites,  SaaS platforms,  professional tools | Creative portfolios,  entertainment,  playful brands,  artistic experiments | supported | 1-4 | 2-5 | 6-10 |
| 2 | Neumorphism | Soft UI,  embossed,  debossed,  convex,  concave,  light source,  subtle depth,  rounded (12-16px),  monochromatic | Health/wellness apps,  meditation platforms,  fitness trackers,  minimal interaction UIs | Complex apps,  critical accessibility,  data-heavy dashboards,  high-contrast required | conditional | 2-6 | 2-6 | 3-7 |
| 3 | Glassmorphism | Frosted glass,  transparent,  blurred background,  layered,  vibrant background,  light source,  depth,  multi-layer | Modern SaaS,  financial dashboards,  high-end corporate,  lifestyle apps,  modal overlays,  navigation | Low-contrast backgrounds,  critical accessibility,  performance-limited,  dark text on dark | supported | 1-4 | 2-5 | 6-10 |
| 4 | Brutalism | Raw,  unpolished,  stark,  high contrast,  plain text,  default fonts,  visible borders,  asymmetric,  anti-design | Design portfolios,  artistic projects,  counter-culture brands,  editorial/media sites,  tech blogs | Corporate environments,  conservative industries,  critical accessibility,  customer-facing professional | supported | 6-10 | 5-9 | 1-5 |
| 5 | 3D & Hyperrealism | Depth,  realistic textures,  3D models,  spatial navigation,  tactile,  skeuomorphic elements,  rich detail,  immersive | Gaming,  product showcase,  immersive experiences,  high-end e-commerce,  architectural viz,  VR/AR | Low-end mobile,  performance-limited,  critical accessibility,  data tables/forms | conditional | 3-7 | 3-6 | 3-6 |
| 6 | Vibrant & Block-based | Bold,  energetic,  playful,  block layout,  geometric shapes,  high color contrast,  duotone,  modern,  energetic | Startups,  creative agencies,  gaming,  social media,  youth-focused,  entertainment,  consumer | Financial institutions,  healthcare,  formal business,  government,  conservative,  elderly | supported | 6-10 | 5-9 | 1-5 |
| 7 | Dark Mode (OLED) | Dark theme,  low light,  high contrast,  deep black,  midnight blue,  eye-friendly,  OLED,  night mode,  power efficient | Night-mode apps,  coding platforms,  entertainment,  eye-strain prevention,  OLED devices,  low-light | Print-first content,  high-brightness outdoor,  color-accuracy-critical | supported | 2-6 | 2-6 | 3-7 |
| 8 | Accessible & Ethical | Accessible,  inclusive interface,  high contrast,  large text (16px+),  keyboard navigation,  screen reader friendly,  accessibility standards aware,  focus state,  semantic | Government,  healthcare,  education,  inclusive products,  large audience,  legal compliance,  public | None - accessibility universal | supported | 3-7 | 3-6 | 3-6 |
| 9 | Claymorphism | Soft 3D,  chunky,  playful,  toy-like,  bubbly,  thick borders (3-4px),  double shadows,  rounded (16-24px) | Educational apps,  children's apps,  SaaS platforms,  creative tools,  fun-focused,  onboarding,  casual games | Formal corporate,  professional services,  data-critical,  serious/medical,  legal apps,  finance | conditional | 1-4 | 2-5 | 6-10 |
| 10 | Aurora UI | Vibrant gradients,  smooth blend,  Northern Lights effect,  mesh gradient,  luminous,  atmospheric,  abstract | Modern SaaS,  creative agencies,  branding,  music platforms,  lifestyle,  premium products,  hero sections | Data-heavy dashboards,  critical accessibility,  content-heavy where distraction issues | supported | 1-4 | 2-5 | 6-10 |
| 11 | Retro-Futurism | Vintage sci-fi,  80s aesthetic,  neon glow,  geometric patterns,  CRT scanlines,  pixel art,  cyberpunk,  synthwave | Gaming,  entertainment,  music platforms,  tech brands,  artistic projects,  nostalgic,  cyberpunk | Conservative industries,  critical accessibility,  professional/corporate,  elderly,  legal/finance | supported | 6-10 | 5-9 | 1-5 |
| 12 | Flat Design | 2D,  minimalist,  bold colors,  no shadows,  clean lines,  simple shapes,  typography-focused,  modern,  icon-heavy | Web apps,  mobile apps,  cross-platform,  startup MVPs,  user-friendly,  SaaS,  dashboards,  corporate | Complex 3D,  premium/luxury,  artistic portfolios,  immersive experiences,  high-detail | supported | 1-4 | 2-5 | 6-10 |
| 13 | Skeuomorphism | Realistic,  texture,  depth,  3D appearance,  real-world metaphors,  shadows,  gradients,  tactile,  detailed,  material | Legacy apps,  gaming,  immersive storytelling,  premium products,  luxury,  realistic simulations,  education | Modern enterprise,  critical accessibility,  low-performance,  web (use Flat/Modern) | conditional | 3-7 | 3-6 | 3-6 |
| 14 | Liquid Glass | dynamic material,  optical glass,  translucency,  lensing,  refraction,  fluid morphing,  system navigation | Apple-platform navigation,  controls,  and system-aligned app chrome | content layers,  dense reading surfaces,  or custom effects without accessibility fallbacks | supported | 2-6 | 2-6 | 3-7 |
| 15 | Motion-Driven | Animation-heavy,  microinteractions,  smooth transitions,  scroll effects,  parallax,  entrance anim,  page transitions | Portfolio sites,  storytelling platforms,  interactive experiences,  entertainment apps,  creative,  SaaS | Data dashboards,  critical accessibility,  low-power devices,  content-heavy,  motion-sensitive | supported | 1-4 | 2-5 | 6-10 |
| 16 | Micro-interactions | Small animations,  gesture-based,  tactile feedback,  subtle animations,  contextual interactions,  responsive | Mobile apps,  touchscreen UIs,  productivity tools,  user-friendly,  consumer apps,  interactive components | Desktop-only,  critical performance,  accessibility-first (alternatives needed) | supported | 3-7 | 3-6 | 3-6 |
| 17 | Inclusive Design | Accessible,  color-blind friendly,  high contrast,  haptic feedback,  voice interaction,  screen reader,  enhanced contrast targets,  universal | Public services,  education,  healthcare,  finance,  government,  accessible consumer,  inclusive | None - accessibility universal | supported | 2-6 | 2-6 | 3-7 |
| 18 | Zero Interface | Minimal visible UI,  voice-first,  gesture-based,  AI-driven,  invisible controls,  predictive,  context-aware,  ambient | Voice assistants,  AI platforms,  future-forward UX,  smart home,  contextual computing,  ambient experiences | Complex workflows,  data-entry heavy,  traditional systems,  legacy support,  explicit control | supported | 6-10 | 5-9 | 1-5 |
| 19 | Soft UI Evolution | Evolved soft UI,  better contrast,  modern aesthetics,  subtle depth,  accessibility-focused,  improved shadows,  hybrid | Modern enterprise apps,  SaaS platforms,  health/wellness,  modern business tools,  professional,  hybrid | Extreme minimalism,  critical performance,  systems without modern OS | supported | 1-4 | 2-5 | 6-10 |
| 20 | Hero-Centric Design | Large hero section,  compelling headline,  high-contrast CTA,  product showcase,  value proposition,  hero image/video,  dramatic visual | SaaS landing pages,  product launches,  service landing pages,  B2B platforms,  tech companies | Complex navigation,  multi-page experiences,  data-heavy applications | supported | 1-4 | 2-5 | 6-10 |
| 21 | Conversion-Optimized | Form-focused,  minimalist design,  single CTA focus,  high contrast,  urgency elements,  trust signals,  social proof,  clear value | E-commerce product pages,  free trial signups,  lead generation,  SaaS pricing pages,  limited-time offers | Complex feature explanations,  multi-product showcases,  technical documentation | supported | 1-4 | 2-5 | 6-10 |
| 22 | Feature-Rich Showcase | Multiple feature sections,  grid layout,  benefit cards,  visual feature demonstrations,  interactive elements,  problem-solution pairs | Enterprise SaaS,  software tools landing pages,  platform services,  complex product explanations,  B2B products | Simple product pages,  early-stage startups with few features,  entertainment landing pages | supported | 1-4 | 2-5 | 6-10 |
| 23 | Minimal & Direct | Minimal text,  white space heavy,  single column layout,  direct messaging,  clean typography,  visual-centric,  fast-loading | Simple service landing pages,  indie products,  consulting services,  micro SaaS,  freelancer portfolios | Feature-heavy products,  complex explanations,  multi-product showcases | supported | 1-4 | 2-5 | 6-10 |
| 24 | Social Proof-Focused | Testimonials prominent,  client logos displayed,  case studies sections,  reviews/ratings,  user avatars,  success metrics,  credibility markers | B2B SaaS,  professional services,  premium products,  e-commerce conversion pages,  established brands | Startup MVPs,  products without users,  niche/experimental products | supported | 1-4 | 2-5 | 6-10 |
| 25 | Interactive Product Demo | Embedded product mockup/video,  interactive elements,  product walkthrough,  step-by-step guides,  hover-to-reveal features,  embedded demos | SaaS platforms,  tool/software products,  productivity apps landing pages,  developer tools,  productivity software | Simple services,  consulting,  non-digital products,  complexity-averse audiences | supported | 1-4 | 2-5 | 6-10 |
| 26 | Trust & Authority | Certificates/badges displayed,  expert credentials,  case studies with metrics,  before/after comparisons,  industry recognition,  security badges | Healthcare/medical landing pages,  financial services,  enterprise software,  premium/luxury products,  legal services | Casual products,  entertainment,  viral/social-first products | supported | 1-4 | 2-5 | 6-10 |
| 27 | Storytelling-Driven | Narrative flow,  visual story progression,  section transitions,  consistent character/brand voice,  emotional messaging,  journey visualization | Brand/startup stories,  mission-driven products,  premium/lifestyle brands,  documentary-style products,  educational | Technical/complex products (unless narrative-driven),  traditional enterprise software | supported | 6-10 | 5-9 | 1-5 |
| 28 | Data-Dense Dashboard | Multiple charts/widgets,  data tables,  KPI cards,  minimal padding,  grid layout,  space-efficient,  maximum data visibility | Business intelligence dashboards,  financial analytics,  enterprise reporting,  operational dashboards,  data warehousing | Marketing dashboards,  consumer-facing analytics,  simple reporting | supported | 1-4 | 2-5 | 6-10 |
| 29 | Heat Map & Heatmap Style | Color-coded grid/matrix,  data intensity visualization,  geographical heat maps,  correlation matrices,  cell-based representation,  gradient coloring | Geographical analysis,  performance matrices,  correlation analysis,  user behavior heatmaps,  temperature/intensity data | Linear data representation,  categorical comparisons (use bar charts),  small datasets | supported | 1-4 | 2-5 | 6-10 |
| 30 | Executive Dashboard | High-level KPIs,  large key metrics,  minimal detail,  summary view,  trend indicators,  at-a-glance insights,  executive summary | C-suite dashboards,  business summary reports,  decision-maker dashboards,  strategic planning views | Detailed analyst dashboards,  technical deep-dives,  operational monitoring | supported | 1-4 | 2-5 | 6-10 |
| 31 | Real-Time Monitoring | Live data updates,  status indicators,  alert notifications,  streaming data visualization,  active monitoring,  streaming charts | System monitoring dashboards,  DevOps dashboards,  real-time analytics,  stock market dashboards,  live event tracking | Historical analysis,  long-term trend reports,  archived data dashboards | supported | 1-4 | 2-5 | 6-10 |
| 32 | Drill-Down Analytics | Hierarchical data exploration,  expandable sections,  interactive drill-down paths,  summary-to-detail flow,  context preservation | Sales analytics,  product analytics,  funnel analysis,  multi-dimensional data exploration,  business intelligence | Simple linear data,  single-metric dashboards,  streaming real-time dashboards | supported | 1-4 | 2-5 | 6-10 |
| 33 | Comparative Analysis Dashboard | Side-by-side comparisons,  period-over-period metrics,  A/B test results,  regional comparisons,  performance benchmarks | Period-over-period reporting,  A/B test dashboards,  market comparison,  competitive analysis,  regional performance | Single metric dashboards,  future projections (use forecasting),  real-time only (no historical) | supported | 1-4 | 2-5 | 6-10 |
| 34 | Predictive Analytics | Forecast lines,  confidence intervals,  trend projections,  scenario modeling,  AI-driven insights,  anomaly detection visualization | Forecasting dashboards,  anomaly detection systems,  trend prediction dashboards,  AI-powered analytics,  budget planning | Historical-only dashboards,  simple reporting,  real-time operational dashboards | supported | 1-4 | 2-5 | 6-10 |
| 35 | User Behavior Analytics | Funnel visualization,  user flow diagrams,  conversion tracking,  engagement metrics,  user journey mapping,  cohort analysis | Conversion funnel analysis,  user journey tracking,  engagement analytics,  cohort analysis,  retention tracking | Real-time operational metrics,  technical system monitoring,  financial transactions | supported | 1-4 | 2-5 | 6-10 |
| 36 | Financial Dashboard | Revenue metrics,  profit/loss visualization,  budget tracking,  financial ratios,  portfolio performance,  cash flow,  audit trail | Financial reporting,  accounting dashboards,  portfolio tracking,  budget monitoring,  banking analytics | Simple business dashboards,  entertainment/social metrics,  non-financial data | supported | 1-4 | 2-5 | 6-10 |
| 37 | Sales Intelligence Dashboard | Deal pipeline,  sales metrics,  territory performance,  sales rep leaderboard,  win-loss analysis,  quota tracking,  forecast accuracy | CRM dashboards,  sales management,  opportunity tracking,  performance management,  quota planning | Marketing analytics,  customer support metrics,  HR dashboards | supported | 1-4 | 2-5 | 6-10 |
| 38 | Neubrutalism | Bold borders,  black outlines,  primary colors,  thick shadows,  no gradients,  flat colors,  45° shadows,  playful,  Gen Z | Gen Z brands,  startups,  creative agencies,  Figma-style apps,  Notion-style interfaces,  tech blogs | Luxury brands,  finance,  healthcare,  conservative industries (too playful) | supported | 6-10 | 5-9 | 1-5 |
| 39 | Bento Box Grid | Modular cards,  asymmetric grid,  varied sizes,  Apple-style,  dashboard tiles,  negative space,  clean hierarchy,  cards | Dashboards,  product pages,  portfolios,  Apple-style marketing,  feature showcases,  SaaS | Dense data tables,  text-heavy content,  real-time monitoring | supported | 1-4 | 2-5 | 6-10 |
| 40 | Y2K Aesthetic | Neon pink,  chrome,  metallic,  bubblegum,  iridescent,  glossy,  retro-futurism,  2000s,  futuristic nostalgia | Fashion brands,  music platforms,  Gen Z brands,  nostalgia marketing,  entertainment,  youth-focused | B2B enterprise,  healthcare,  finance,  conservative industries,  elderly users | conditional | 6-10 | 5-9 | 1-5 |
| 41 | Cyberpunk UI | Neon,  dark mode,  terminal,  HUD,  sci-fi,  glitch,  dystopian,  futuristic,  matrix,  tech noir | Gaming platforms,  tech products,  crypto apps,  sci-fi applications,  developer tools,  entertainment | Corporate enterprise,  healthcare,  family apps,  conservative brands,  elderly users | supported | 3-7 | 3-6 | 3-6 |
| 42 | Organic Biophilic | Nature,  organic shapes,  green,  sustainable,  rounded,  flowing,  wellness,  earthy,  natural textures | Wellness apps,  sustainability brands,  eco products,  health apps,  meditation,  organic food brands | Tech-focused products,  gaming,  industrial,  urban brands | supported | 3-7 | 3-6 | 3-6 |
| 43 | AI-Native UI | Chatbot,  conversational,  voice,  assistant,  agentic,  ambient,  minimal chrome,  streaming text,  AI interactions | AI products,  chatbots,  voice assistants,  copilots,  AI-powered tools,  conversational interfaces | Traditional forms,  data-heavy dashboards,  print-first content | supported | 3-7 | 3-6 | 3-6 |
| 44 | Memphis Design | 80s,  geometric,  playful,  postmodern,  shapes,  patterns,  squiggles,  triangles,  neon,  abstract,  bold | Creative agencies,  music sites,  youth brands,  event promotion,  artistic portfolios,  entertainment | Corporate finance,  healthcare,  legal,  elderly users,  conservative brands | supported | 6-10 | 5-9 | 1-5 |
| 45 | Vaporwave | Synthwave,  retro-futuristic,  80s-90s,  neon,  glitch,  nostalgic,  sunset gradient,  dreamy,  aesthetic | Music platforms,  gaming,  creative portfolios,  tech startups,  entertainment,  artistic projects | Business apps,  e-commerce,  education,  healthcare,  enterprise software | supported | 6-10 | 5-9 | 1-5 |
| 46 | Dimensional Layering | Depth,  overlapping,  z-index,  layers,  3D,  shadows,  elevation,  floating,  cards,  spatial hierarchy | Dashboards,  card layouts,  modals,  navigation,  product showcases,  SaaS interfaces | Print-style layouts,  simple blogs,  low-end devices,  flat design requirements | supported | 1-4 | 2-5 | 6-10 |
| 47 | Exaggerated Minimalism | Bold minimalism,  oversized typography,  high contrast,  negative space,  loud minimal,  statement design | Fashion,  architecture,  portfolios,  agency landing pages,  luxury brands,  editorial | E-commerce catalogs,  dashboards,  forms,  data-heavy,  elderly users,  complex apps | supported | 6-10 | 5-9 | 1-5 |
| 48 | Kinetic Typography | Motion text,  animated type,  moving letters,  dynamic,  typing effect,  morphing,  scroll-triggered text | Hero sections,  marketing sites,  video platforms,  storytelling,  creative portfolios,  landing pages | Long-form content,  accessibility-critical,  data interfaces,  forms,  elderly users | supported | 6-10 | 5-9 | 1-5 |
| 49 | Parallax Storytelling | Scroll-driven,  narrative,  layered scrolling,  immersive,  progressive disclosure,  cinematic,  scroll-triggered | Brand storytelling,  product launches,  case studies,  portfolios,  annual reports,  marketing campaigns | E-commerce,  dashboards,  mobile-first,  SEO-critical,  accessibility-required | supported | 6-10 | 5-9 | 1-5 |
| 50 | Swiss Modernism 2.0 | Grid system,  Helvetica,  modular,  asymmetric,  international style,  rational,  clean,  mathematical spacing | Corporate sites,  architecture,  editorial,  SaaS,  museums,  professional services,  documentation | Playful brands,  children's sites,  entertainment,  gaming,  emotional storytelling | supported | 1-4 | 2-5 | 6-10 |
| 51 | HUD / Sci-Fi FUI | Futuristic,  technical,  wireframe,  neon,  data,  transparency,  iron man,  sci-fi,  interface | Sci-fi games,  space tech,  cybersecurity,  movie props,  immersive dashboards | Standard corporate,  reading heavy content,  accessible public services | supported | 1-4 | 2-5 | 6-10 |
| 52 | Pixel Art | Retro,  8-bit,  16-bit,  gaming,  blocky,  nostalgic,  pixelated,  arcade | Indie games,  retro tools,  creative portfolios,  nostalgia marketing,  Web3/NFT | Professional corporate,  modern SaaS,  high-res photography sites | supported | 6-10 | 5-9 | 1-5 |
| 53 | Bento Grids (Legacy) | Apple-style,  modular,  cards,  organized,  clean,  hierarchy,  grid,  rounded,  soft | Product features,  dashboards,  personal sites,  marketing summaries,  galleries | Long-form reading,  data tables,  complex forms | supported | 1-4 | 2-5 | 6-10 |
| 54 | Spatial UI (VisionOS) | Glass,  depth,  immersion,  spatial,  translucent,  gaze,  gesture,  apple,  vision-pro | Spatial computing apps,  VR/AR interfaces,  immersive media,  futuristic dashboards | Text-heavy documents,  high-contrast requirements,  non-3D capable devices | supported | 1-4 | 2-5 | 6-10 |
| 55 | E-Ink / Paper | Paper-like,  matte,  high contrast,  texture,  reading,  calm,  slow tech,  monochrome | Reading apps,  digital newspapers,  minimal journals,  distraction-free writing,  slow-living brands | Gaming,  video platforms,  high-energy marketing,  dark mode dependent apps | not-recommended | 1-4 | 1-4 | 4-7 |
| 56 | Gen Z Chaos / Maximalism | Chaos,  clutter,  stickers,  raw,  collage,  mixed media,  loud,  internet culture,  ironic | Gen Z lifestyle brands,  music artists,  creative portfolios,  viral marketing,  fashion | Corporate,  government,  healthcare,  banking,  serious tools | supported | 6-10 | 5-9 | 1-5 |
| 57 | Biomimetic / Organic 2.0 | Nature-inspired,  cellular,  fluid,  breathing,  generative,  algorithms,  life-like | Sustainability tech,  biotech,  advanced health,  meditation,  generative art platforms | Standard SaaS,  data grids,  strict corporate,  accounting | supported | 6-10 | 5-9 | 1-5 |
| 58 | Anti-Polish / Raw Aesthetic | Hand-drawn,  collage,  scanned textures,  unfinished,  imperfect,  authentic,  human,  sketch,  raw marks,  creative process | Creative portfolios,  artist sites,  indie brands,  handmade products,  authentic storytelling,  editorial | Corporate enterprise,  fintech,  healthcare,  government,  polished SaaS | supported | 6-10 | 5-9 | 1-5 |
| 59 | Tactile Digital / Deformable UI | Jelly buttons,  chrome,  clay,  squishy,  deformable,  bouncy,  physical,  tactile feedback,  press response | Modern mobile apps,  playful brands,  entertainment,  gaming UI,  consumer products,  interactive demos | Enterprise software,  data dashboards,  accessibility-critical,  professional tools | supported | 3-7 | 3-6 | 3-6 |
| 60 | Nature Distilled | Muted earthy,  skin tones,  wood,  soil,  sand,  terracotta,  warmth,  organic materials,  handmade warmth | Wellness brands,  sustainable products,  artisan goods,  organic food,  spa/beauty,  home decor | Tech startups,  gaming,  nightlife,  corporate finance,  high-energy brands | conditional | 6-10 | 5-9 | 1-5 |
| 61 | Interactive Cursor Design | Custom cursor,  cursor as tool,  hover effects,  cursor feedback,  pointer transformation,  cursor trail,  magnetic cursor | Creative portfolios,  interactive experiences,  agency sites,  product showcases,  gaming,  entertainment | Mobile-first (no cursor),  accessibility-critical,  data-heavy dashboards,  forms | supported | 6-10 | 5-9 | 1-5 |
| 62 | Voice-First Multimodal | Voice UI,  multimodal,  audio feedback,  conversational,  hands-free,  ambient,  contextual,  speech recognition | Voice assistants,  accessibility apps,  hands-free tools,  smart home,  automotive UI,  cooking apps | Visual-heavy content,  data entry,  complex forms,  noisy environments | supported | 6-10 | 5-9 | 1-5 |
| 63 | 3D Product Preview | 360 product view,  rotatable,  zoomable,  touch-to-spin,  AR preview,  product configurator,  interactive 3D model | E-commerce,  furniture,  fashion,  automotive,  electronics,  jewelry,  product configurators | Content-heavy sites,  blogs,  dashboards,  low-bandwidth,  accessibility-critical | conditional | 6-10 | 5-9 | 1-5 |
| 64 | Gradient Mesh / Aurora Evolved | Complex gradients,  mesh gradients,  multi-color blend,  aurora effect,  flowing colors,  iridescent,  holographic,  prismatic | Hero sections,  backgrounds,  creative brands,  music platforms,  fashion,  lifestyle,  premium products | Data interfaces,  text-heavy content,  accessibility-critical,  conservative brands | supported | 6-10 | 5-9 | 1-5 |
| 65 | Editorial Grid / Magazine | Magazine layout,  asymmetric grid,  editorial typography,  pull quotes,  drop caps,  column layout,  print-inspired | News sites,  blogs,  magazines,  editorial content,  long-form articles,  journalism,  publishing | Dashboards,  apps,  e-commerce catalogs,  real-time data,  short-form content | supported | 6-10 | 5-9 | 1-5 |
| 66 | Chromatic Aberration / RGB Split | RGB split,  color fringing,  glitch,  retro tech,  VHS,  analog error,  distortion,  lens effect | Music platforms,  gaming,  tech brands,  creative portfolios,  nightlife,  entertainment,  video platforms | Corporate,  healthcare,  finance,  accessibility-critical,  elderly users | supported | 6-10 | 5-9 | 1-5 |
| 67 | Vintage Analog / Retro Film | Film grain,  VHS,  cassette tape,  polaroid,  analog warmth,  faded colors,  light leaks,  vintage photography | Photography portfolios,  music/vinyl brands,  vintage fashion,  nostalgia marketing,  film industry,  cafes | Modern tech,  SaaS,  healthcare,  children's apps,  corporate enterprise | conditional | 6-10 | 5-9 | 1-5 |
| 68 | Bauhaus (包豪斯) | bauhaus,  geometric,  constructivist,  primary colors,  hard shadow,  bold,  tactile,  functional,  poster,  mechanical,  architectural | Mobile-first apps needing high personality,  onboarding flows,  branding-forward product screens,  artisan/design brands,  editorial mobile experiences | Enterprise dashboards,  accessibility-critical contexts (requires extra a11y work),  data-heavy screens,  conservative industries | conditional | 6-10 | 5-9 | 1-5 |
| 69 | Minimalist Monochrome | monochrome,  black white,  editorial,  austere,  typographic,  sharp,  zero radius,  high contrast,  brutalist,  pocket editorial,  serif,  mechanical | Luxury fashion e-commerce mobile,  editorial publications,  high-end portfolio apps,  experimental/avant-garde brands,  digital exhibitions | Entertainment,  colorful brands,  friendly consumer apps,  anything requiring visual warmth or gradient | conditional | 6-10 | 5-9 | 1-5 |
| 70 | Modern Dark (Cinema Mobile) | dark mode,  cinematic,  ambient light,  glassmorphism,  deep black,  indigo,  glow,  blur,  atmospheric,  reanimated,  haptic,  premium,  layered,  frosted glass,  linear gradient | Developer tools,  pro productivity apps,  fintech/trading dashboards,  media/streaming platforms,  AI tool interfaces,  high-end gaming companion apps | Consumer apps needing warmth,  children's apps,  health/medical contexts where dark feels harsh,  high-accessibility contexts needing maximum contrast | supported | 1-4 | 2-5 | 6-10 |
| 71 | SaaS Mobile (High-Tech Boutique) | saas,  electric blue,  gradient,  fintech,  spring animation,  dual font,  glassmorphism,  boutique,  premium,  calistoga,  inter,  mono,  tactile,  haptic,  bento | B2B SaaS mobile dashboards,  fintech apps,  developer tool mobile companions,  marketing analytics apps,  HR/operations apps,  modern business productivity | Pure consumer entertainment,  children's apps,  highly decorative lifestyle apps,  contexts where Electric Blue feels too corporate | conditional | 1-4 | 2-5 | 6-10 |
| 72 | Terminal CLI (Mobile) | terminal,  cli,  matrix green,  monospace,  hacker,  ascii,  command line,  developer,  web3,  crypto,  sci-fi,  OLED,  retro-future,  field operative | Developer tools,  Web3/blockchain apps,  geek-culture apps,  ARG games,  sci-fi/noir gaming companions,  hacker/security tools,  creative studio portfolios | Consumer products,  health apps,  anything requiring approachability or warmth,  children's apps,  standard enterprise contexts | supported | 6-10 | 5-9 | 1-5 |
| 73 | Kinetic Brutalism (Mobile) | kinetic,  brutalism,  motion,  marquee,  acid yellow,  uppercase,  oversized,  aggressive typography,  street,  zine,  high contrast,  scroll-driven,  haptic,  reanimated | Immersive storytelling apps,  brand flagship mobile,  music/culture platforms,  sports apps,  underground zines,  limited-edition product drops,  performance dashboards | Calm informational apps,  healthcare,  finance contexts needing trust,  children's,  any context where aggressive typography feels inappropriate | conditional | 1-4 | 2-5 | 6-10 |
| 74 | Flat Design Mobile (Touch-First) | flat,  2D,  no shadow,  color blocking,  geometric,  bold,  poster,  icon,  touch-first,  minimal,  clean,  tailored,  cross-platform | Cross-platform apps (iOS+Android parity),  information-dense dashboards,  system UI,  brand illustration,  onboarding flows,  marketing pages,  icon design | Ultra-premium contexts needing depth/shadow,  dark-mode-first products,  contexts where flat design reads as unfinished or sterile | conditional | 1-4 | 2-5 | 6-10 |
| 75 | Material 3 Expressive (Mobile) | material 3 expressive,  vibrant color,  spring motion,  adaptive components,  flexible typography,  contrasting shapes,  android | Android,  Wear OS,  and Pixel-aligned products using Material 3 components | Ultra-minimal brutalist brands,  terminal/hacker aesthetics,  monochrome editorial apps | supported | 3-7 | 3-6 | 3-6 |
| 76 | Neo Brutalism (Mobile) | neo brutalism,  pop art,  stickers,  thick borders,  cream background,  hot red,  vivid yellow,  soft violet,  hard offset shadow,  mechanical press,  collage | Creative tools,  collab platforms,  Gen Z marketing & e-commerce,  portfolio sites,  sticker-book style content apps | Serious enterprise apps,  conservative industries,  sober fintech,  accessibility-first contexts (must tune contrast) | not-recommended | 6-10 | 5-9 | 1-5 |
| 77 | Bold Typography (Mobile Poster) | bold typography,  editorial,  poster,  broadsheet,  vermillion,  negative space,  edge-to-edge type,  underline CTA,  near-black,  warm white | Creative brand heroes,  reading-focused apps,  event/exhibition pages,  editorial mobile experiences,  landing hero sections | Utility dashboards,  kids apps,  playful consumer products,  contexts needing many icons or heavy imagery | conditional | 6-10 | 5-9 | 1-5 |
| 78 | Academia (Scholarly Mobile) | academia,  library,  mahogany,  parchment,  brass,  crimson,  serif,  drop cap,  arch-top,  vignette,  leather,  scholarly,  tactile | Knowledge management apps,  deep reading tools,  ritual-heavy personal brands,  lore-heavy RPG/roleplay apps,  culture-specific community platforms | Hyper-modern tech dashboards,  neon/glassmorphism,  playful Gen Z branding | conditional | 2-6 | 2-6 | 3-7 |
| 79 | Cyberpunk Mobile HUD | cyberpunk,  neon,  glitch,  chamfered,  orbitron,  jetbrains,  scanlines,  crt,  hud,  matrix,  military,  decker | Gaming dashboards,  crypto/cyberpunk apps,  sci-fi companion tools,  hacker OS skins,  data-heavy monitoring HUDs | Serious enterprise,  health/finance requiring calm trust,  minimal editorial apps | supported | 1-4 | 2-5 | 6-10 |
| 80 | Bitcoin DeFi (Mobile) | web3,  bitcoin,  defi,  digital gold,  fintech,  wallet,  orange,  glassmorphism,  gradient,  blur,  holographic,  trust,  precision | DeFi dashboards,  wallets,  NFT marketplaces,  Web3 social,  metaverse utilities,  high-tech fintech brands | Playful casual apps,  low-tech brands,  ultra-minimal editorial apps | supported | 1-4 | 2-5 | 6-10 |
| 81 | Claymorphism (Mobile) | claymorphism,  clay,  3d,  soft,  bubbly,  candy,  playful,  rounded,  squish,  tactile,  inflate,  silicone,  haptic,  spring | Children education apps,  teen social products,  crypto gamification,  creative tools,  brand mascot-led apps | Serious enterprise,  high-density data,  editorial reading apps,  fintech trust signals | supported | 6-10 | 5-9 | 1-5 |
| 82 | Enterprise SaaS (Mobile) | enterprise,  saas,  b2b,  professional,  indigo,  violet,  gradient,  polished,  trustworthy,  clean,  approachable,  spring,  haptic | B2B backend management,  productivity tools,  government and finance mobile apps,  SaaS companion apps,  enterprise dashboards | Pure consumer entertainment,  Gen-Z youth apps,  gaming UI,  ultra-minimal editorial | supported | 1-4 | 2-5 | 6-10 |
| 83 | Sketch Hand-Drawn (Mobile) | sketch,  hand-drawn,  handwriting,  wobbly,  imperfect,  paper,  kalam,  organic,  collage,  post-it,  tape,  offset shadow,  scribble | Low-fidelity prototyping,  creative brands,  children/picturebook apps,  education tools,  journaling apps,  gamified puzzles | Enterprise dashboards,  high-density data tables,  fintech precision tools,  medical or legal apps | supported | 6-10 | 5-9 | 1-5 |
| 84 | Neumorphism (Mobile) | neumorphism,  soft ui,  dual shadow,  extruded,  inset,  clay surface,  monochromatic,  cool grey,  haptic,  ceramic,  physical,  depth | Minimal hardware controls,  smart home apps,  aesthetic utility tools,  health monitors,  brand showcase pages | High-density data,  bright multi-color apps,  apps needing strong visual hierarchy via color,  dark-mode-only products | not-recommended | 6-10 | 5-9 | 1-5 |
| 85 | Fluent 2 | fluent 2,  microsoft,  enterprise,  calm,  rounded,  tokenized,  cross-platform,  copilot | Microsoft 365,  Windows,  Copilot,  and enterprise line-of-business tools | Products that should not inherit Microsoft platform conventions | supported | 1-4 | 2-5 | 6-10 |
| 86 | Shopify Polaris | shopify polaris,  merchant admin,  commerce,  checkout,  web components,  app home | Shopify admin apps,  merchant tools,  checkout,  customer accounts,  POS,  and extensions | Generic marketing sites or products outside Shopify surfaces | supported | 2-5 | 3-5 | 4-7 |
| 87 | Adobe Spectrum | adobe spectrum,  creative tools,  enterprise,  content creation,  tokenized,  cross-platform | Creative tools,  media workflows,  document products,  and Adobe-adjacent enterprise software | Consumer brands that do not need dense professional-tool conventions | supported | 1-4 | 2-5 | 6-10 |
| 88 | Spectrum 2 | spectrum 2,  adobe,  expressive,  approachable,  adaptive,  inclusive,  creative tools | New Adobe-style creative and document surfaces adopting Spectrum 2 | Products not aligned with Adobe professional workflows | supported | 6-10 | 5-9 | 1-5 |

## Best-Fit Variants (173 entries)

Each variant scopes a base style to one use case; dial ranges override the base row when they differ from direction intent.

| # | Base Style | Use Case | VARIANCE | MOTION | DENSITY | Dark Mode |
|---|-----------|----------|----------|--------|---------|-----------|
| 1 | Minimalism & Swiss Style | Enterprise apps | 1-4 | 2-5 | 6-10 | supported |
| 2 | Minimalism & Swiss Style | dashboards | 1-4 | 2-5 | 6-10 | supported |
| 3 | Neumorphism | Health | 2-6 | 2-6 | 3-7 | conditional |
| 4 | Neumorphism | wellness apps | 2-6 | 2-6 | 3-7 | conditional |
| 5 | Glassmorphism | Modern SaaS | 1-4 | 2-5 | 6-10 | supported |
| 6 | Glassmorphism | financial dashboards | 1-4 | 2-5 | 6-10 | supported |
| 7 | Brutalism | Design portfolios | 6-10 | 5-9 | 1-5 | supported |
| 8 | Brutalism | artistic projects | 6-10 | 5-9 | 1-5 | supported |
| 9 | 3D & Hyperrealism | Gaming | 3-7 | 3-6 | 3-6 | conditional |
| 10 | 3D & Hyperrealism | product showcase | 3-7 | 3-6 | 3-6 | conditional |
| 11 | Vibrant & Block-based | Startups | 6-10 | 5-9 | 1-5 | supported |
| 12 | Vibrant & Block-based | creative agencies | 6-10 | 5-9 | 1-5 | supported |
| 13 | Dark Mode (OLED) | Night-mode apps | 2-6 | 2-6 | 3-7 | supported |
| 14 | Dark Mode (OLED) | coding platforms | 2-6 | 2-6 | 3-7 | supported |
| 15 | Accessible & Ethical | Government | 3-7 | 3-6 | 3-6 | supported |
| 16 | Accessible & Ethical | healthcare | 3-7 | 3-6 | 3-6 | supported |
| 17 | Claymorphism | Educational apps | 1-4 | 2-5 | 6-10 | conditional |
| 18 | Claymorphism | children's apps | 1-4 | 2-5 | 6-10 | conditional |
| 19 | Aurora UI | Modern SaaS | 1-4 | 2-5 | 6-10 | supported |
| 20 | Aurora UI | creative agencies | 1-4 | 2-5 | 6-10 | supported |
| 21 | Retro-Futurism | Gaming | 6-10 | 5-9 | 1-5 | supported |
| 22 | Retro-Futurism | entertainment | 6-10 | 5-9 | 1-5 | supported |
| 23 | Flat Design | Web apps | 1-4 | 2-5 | 6-10 | supported |
| 24 | Flat Design | mobile apps | 1-4 | 2-5 | 6-10 | supported |
| 25 | Skeuomorphism | Legacy apps | 3-7 | 3-6 | 3-6 | conditional |
| 26 | Skeuomorphism | gaming | 3-7 | 3-6 | 3-6 | conditional |
| 27 | Liquid Glass | Apple-platform navigation | 2-6 | 2-6 | 3-7 | supported |
| 28 | Liquid Glass | controls | 2-6 | 2-6 | 3-7 | supported |
| 29 | Motion-Driven | Portfolio sites | 1-4 | 2-5 | 6-10 | supported |
| 30 | Motion-Driven | storytelling platforms | 1-4 | 2-5 | 6-10 | supported |
| 31 | Micro-interactions | Mobile apps | 3-7 | 3-6 | 3-6 | supported |
| 32 | Micro-interactions | touchscreen UIs | 3-7 | 3-6 | 3-6 | supported |
| 33 | Inclusive Design | Public services | 2-6 | 2-6 | 3-7 | supported |
| 34 | Inclusive Design | education | 2-6 | 2-6 | 3-7 | supported |
| 35 | Zero Interface | Voice assistants | 6-10 | 5-9 | 1-5 | supported |
| 36 | Zero Interface | AI platforms | 6-10 | 5-9 | 1-5 | supported |
| 37 | Soft UI Evolution | Modern enterprise apps | 1-4 | 2-5 | 6-10 | supported |
| 38 | Soft UI Evolution | SaaS platforms | 1-4 | 2-5 | 6-10 | supported |
| 39 | Hero-Centric Design | SaaS landing pages | 1-4 | 2-5 | 6-10 | supported |
| 40 | Hero-Centric Design | product launches | 1-4 | 2-5 | 6-10 | supported |
| 41 | Conversion-Optimized | E-commerce product pages | 1-4 | 2-5 | 6-10 | supported |
| 42 | Conversion-Optimized | free trial signups | 1-4 | 2-5 | 6-10 | supported |
| 43 | Feature-Rich Showcase | Enterprise SaaS | 1-4 | 2-5 | 6-10 | supported |
| 44 | Feature-Rich Showcase | software tools landing pages | 1-4 | 2-5 | 6-10 | supported |
| 45 | Minimal & Direct | Simple service landing pages | 1-4 | 2-5 | 6-10 | supported |
| 46 | Minimal & Direct | indie products | 1-4 | 2-5 | 6-10 | supported |
| 47 | Social Proof-Focused | B2B SaaS | 1-4 | 2-5 | 6-10 | supported |
| 48 | Social Proof-Focused | professional services | 1-4 | 2-5 | 6-10 | supported |
| 49 | Interactive Product Demo | SaaS platforms | 1-4 | 2-5 | 6-10 | supported |
| 50 | Interactive Product Demo | tool | 1-4 | 2-5 | 6-10 | supported |
| 51 | Trust & Authority | Healthcare | 1-4 | 2-5 | 6-10 | supported |
| 52 | Trust & Authority | medical landing pages | 1-4 | 2-5 | 6-10 | supported |
| 53 | Storytelling-Driven | Brand | 6-10 | 5-9 | 1-5 | supported |
| 54 | Storytelling-Driven | startup stories | 6-10 | 5-9 | 1-5 | supported |
| 55 | Data-Dense Dashboard | Business intelligence dashboards | 1-4 | 2-5 | 6-10 | supported |
| 56 | Data-Dense Dashboard | financial analytics | 1-4 | 2-5 | 6-10 | supported |
| 57 | Heat Map & Heatmap Style | Geographical analysis | 1-4 | 2-5 | 6-10 | supported |
| 58 | Heat Map & Heatmap Style | performance matrices | 1-4 | 2-5 | 6-10 | supported |
| 59 | Executive Dashboard | C-suite dashboards | 1-4 | 2-5 | 6-10 | supported |
| 60 | Executive Dashboard | business summary reports | 1-4 | 2-5 | 6-10 | supported |
| 61 | Real-Time Monitoring | System monitoring dashboards | 1-4 | 2-5 | 6-10 | supported |
| 62 | Real-Time Monitoring | DevOps dashboards | 1-4 | 2-5 | 6-10 | supported |
| 63 | Drill-Down Analytics | Sales analytics | 1-4 | 2-5 | 6-10 | supported |
| 64 | Drill-Down Analytics | product analytics | 1-4 | 2-5 | 6-10 | supported |
| 65 | Comparative Analysis Dashboard | Period-over-period reporting | 1-4 | 2-5 | 6-10 | supported |
| 66 | Predictive Analytics | Forecasting dashboards | 1-4 | 2-5 | 6-10 | supported |
| 67 | Predictive Analytics | anomaly detection systems | 1-4 | 2-5 | 6-10 | supported |
| 68 | User Behavior Analytics | Conversion funnel analysis | 1-4 | 2-5 | 6-10 | supported |
| 69 | User Behavior Analytics | user journey tracking | 1-4 | 2-5 | 6-10 | supported |
| 70 | Financial Dashboard | Financial reporting | 1-4 | 2-5 | 6-10 | supported |
| 71 | Financial Dashboard | accounting dashboards | 1-4 | 2-5 | 6-10 | supported |
| 72 | Sales Intelligence Dashboard | CRM dashboards | 1-4 | 2-5 | 6-10 | supported |
| 73 | Sales Intelligence Dashboard | sales management | 1-4 | 2-5 | 6-10 | supported |
| 74 | Neubrutalism | Gen Z brands | 6-10 | 5-9 | 1-5 | supported |
| 75 | Neubrutalism | startups | 6-10 | 5-9 | 1-5 | supported |
| 76 | Bento Box Grid | Dashboards | 1-4 | 2-5 | 6-10 | supported |
| 77 | Bento Box Grid | product pages | 1-4 | 2-5 | 6-10 | supported |
| 78 | Y2K Aesthetic | Fashion brands | 6-10 | 5-9 | 1-5 | conditional |
| 79 | Y2K Aesthetic | music platforms | 6-10 | 5-9 | 1-5 | conditional |
| 80 | Cyberpunk UI | Gaming platforms | 3-7 | 3-6 | 3-6 | supported |
| 81 | Cyberpunk UI | tech products | 3-7 | 3-6 | 3-6 | supported |
| 82 | Organic Biophilic | Wellness apps | 3-7 | 3-6 | 3-6 | supported |
| 83 | Organic Biophilic | sustainability brands | 3-7 | 3-6 | 3-6 | supported |
| 84 | AI-Native UI | AI products | 3-7 | 3-6 | 3-6 | supported |
| 85 | AI-Native UI | chatbots | 3-7 | 3-6 | 3-6 | supported |
| 86 | Memphis Design | Creative agencies | 6-10 | 5-9 | 1-5 | supported |
| 87 | Memphis Design | music sites | 6-10 | 5-9 | 1-5 | supported |
| 88 | Vaporwave | Music platforms | 6-10 | 5-9 | 1-5 | supported |
| 89 | Vaporwave | gaming | 6-10 | 5-9 | 1-5 | supported |
| 90 | Dimensional Layering | Dashboards | 1-4 | 2-5 | 6-10 | supported |
| 91 | Dimensional Layering | card layouts | 1-4 | 2-5 | 6-10 | supported |
| 92 | Exaggerated Minimalism | Fashion | 6-10 | 5-9 | 1-5 | supported |
| 93 | Exaggerated Minimalism | architecture | 6-10 | 5-9 | 1-5 | supported |
| 94 | Kinetic Typography | Hero sections | 6-10 | 5-9 | 1-5 | supported |
| 95 | Kinetic Typography | marketing sites | 6-10 | 5-9 | 1-5 | supported |
| 96 | Parallax Storytelling | Brand storytelling | 6-10 | 5-9 | 1-5 | supported |
| 97 | Parallax Storytelling | product launches | 6-10 | 5-9 | 1-5 | supported |
| 98 | Swiss Modernism 2.0 | Corporate sites | 1-4 | 2-5 | 6-10 | supported |
| 99 | Swiss Modernism 2.0 | architecture | 1-4 | 2-5 | 6-10 | supported |
| 100 | HUD / Sci-Fi FUI | Sci-fi games | 1-4 | 2-5 | 6-10 | supported |
| 101 | HUD / Sci-Fi FUI | space tech | 1-4 | 2-5 | 6-10 | supported |
| 102 | Pixel Art | Indie games | 6-10 | 5-9 | 1-5 | supported |
| 103 | Pixel Art | retro tools | 6-10 | 5-9 | 1-5 | supported |
| 104 | Bento Grids (Legacy) | Product features | 1-4 | 2-5 | 6-10 | supported |
| 105 | Bento Grids (Legacy) | dashboards | 1-4 | 2-5 | 6-10 | supported |
| 106 | Spatial UI (VisionOS) | Spatial computing apps | 1-4 | 2-5 | 6-10 | supported |
| 107 | E-Ink / Paper | Reading apps | 1-4 | 1-4 | 4-7 | not-recommended |
| 108 | E-Ink / Paper | digital newspapers | 1-4 | 1-4 | 4-7 | not-recommended |
| 109 | Gen Z Chaos / Maximalism | Gen Z lifestyle brands | 6-10 | 5-9 | 1-5 | supported |
| 110 | Gen Z Chaos / Maximalism | music artists | 6-10 | 5-9 | 1-5 | supported |
| 111 | Biomimetic / Organic 2.0 | Sustainability tech | 6-10 | 5-9 | 1-5 | supported |
| 112 | Biomimetic / Organic 2.0 | biotech | 6-10 | 5-9 | 1-5 | supported |
| 113 | Anti-Polish / Raw Aesthetic | Creative portfolios | 6-10 | 5-9 | 1-5 | supported |
| 114 | Anti-Polish / Raw Aesthetic | artist sites | 6-10 | 5-9 | 1-5 | supported |
| 115 | Tactile Digital / Deformable UI | Modern mobile apps | 3-7 | 3-6 | 3-6 | supported |
| 116 | Tactile Digital / Deformable UI | playful brands | 3-7 | 3-6 | 3-6 | supported |
| 117 | Nature Distilled | Wellness brands | 6-10 | 5-9 | 1-5 | conditional |
| 118 | Nature Distilled | sustainable products | 6-10 | 5-9 | 1-5 | conditional |
| 119 | Interactive Cursor Design | Creative portfolios | 6-10 | 5-9 | 1-5 | supported |
| 120 | Interactive Cursor Design | interactive experiences | 6-10 | 5-9 | 1-5 | supported |
| 121 | Voice-First Multimodal | Voice assistants | 6-10 | 5-9 | 1-5 | supported |
| 122 | Voice-First Multimodal | accessibility apps | 6-10 | 5-9 | 1-5 | supported |
| 123 | 3D Product Preview | E-commerce | 6-10 | 5-9 | 1-5 | conditional |
| 124 | 3D Product Preview | furniture | 6-10 | 5-9 | 1-5 | conditional |
| 125 | Gradient Mesh / Aurora Evolved | Hero sections | 6-10 | 5-9 | 1-5 | supported |
| 126 | Gradient Mesh / Aurora Evolved | backgrounds | 6-10 | 5-9 | 1-5 | supported |
| 127 | Editorial Grid / Magazine | News sites | 6-10 | 5-9 | 1-5 | supported |
| 128 | Editorial Grid / Magazine | blogs | 6-10 | 5-9 | 1-5 | supported |
| 129 | Chromatic Aberration / RGB Split | Music platforms | 6-10 | 5-9 | 1-5 | supported |
| 130 | Chromatic Aberration / RGB Split | gaming | 6-10 | 5-9 | 1-5 | supported |
| 131 | Vintage Analog / Retro Film | Photography portfolios | 6-10 | 5-9 | 1-5 | conditional |
| 132 | Vintage Analog / Retro Film | music | 6-10 | 5-9 | 1-5 | conditional |
| 133 | Bauhaus (包豪斯) | Mobile-first apps needing high personality | 6-10 | 5-9 | 1-5 | conditional |
| 134 | Bauhaus (包豪斯) | onboarding flows | 6-10 | 5-9 | 1-5 | conditional |
| 135 | Minimalist Monochrome | Luxury fashion e-commerce mobile | 6-10 | 5-9 | 1-5 | conditional |
| 136 | Minimalist Monochrome | editorial publications | 6-10 | 5-9 | 1-5 | conditional |
| 137 | Modern Dark (Cinema Mobile) | Developer tools | 1-4 | 2-5 | 6-10 | supported |
| 138 | Modern Dark (Cinema Mobile) | pro productivity apps | 1-4 | 2-5 | 6-10 | supported |
| 139 | SaaS Mobile (High-Tech Boutique) | B2B SaaS mobile dashboards | 1-4 | 2-5 | 6-10 | conditional |
| 140 | SaaS Mobile (High-Tech Boutique) | fintech apps | 1-4 | 2-5 | 6-10 | conditional |
| 141 | Terminal CLI (Mobile) | Developer tools | 6-10 | 5-9 | 1-5 | supported |
| 142 | Terminal CLI (Mobile) | Web3 | 6-10 | 5-9 | 1-5 | supported |
| 143 | Kinetic Brutalism (Mobile) | Immersive storytelling apps | 1-4 | 2-5 | 6-10 | conditional |
| 144 | Kinetic Brutalism (Mobile) | brand flagship mobile | 1-4 | 2-5 | 6-10 | conditional |
| 145 | Flat Design Mobile (Touch-First) | Cross-platform apps (iOS+Android parity) | 1-4 | 2-5 | 6-10 | conditional |
| 146 | Flat Design Mobile (Touch-First) | information-dense dashboards | 1-4 | 2-5 | 6-10 | conditional |
| 147 | Material 3 Expressive (Mobile) | Android | 3-7 | 3-6 | 3-6 | supported |
| 148 | Material 3 Expressive (Mobile) | Wear OS | 3-7 | 3-6 | 3-6 | supported |
| 149 | Neo Brutalism (Mobile) | Creative tools | 6-10 | 5-9 | 1-5 | not-recommended |
| 150 | Neo Brutalism (Mobile) | collab platforms | 6-10 | 5-9 | 1-5 | not-recommended |
| 151 | Bold Typography (Mobile Poster) | Creative brand heroes | 6-10 | 5-9 | 1-5 | conditional |
| 152 | Bold Typography (Mobile Poster) | reading-focused apps | 6-10 | 5-9 | 1-5 | conditional |
| 153 | Academia (Scholarly Mobile) | Knowledge management apps | 2-6 | 2-6 | 3-7 | conditional |
| 154 | Academia (Scholarly Mobile) | deep reading tools | 2-6 | 2-6 | 3-7 | conditional |
| 155 | Cyberpunk Mobile HUD | Gaming dashboards | 1-4 | 2-5 | 6-10 | supported |
| 156 | Cyberpunk Mobile HUD | crypto | 1-4 | 2-5 | 6-10 | supported |
| 157 | Bitcoin DeFi (Mobile) | DeFi dashboards | 1-4 | 2-5 | 6-10 | supported |
| 158 | Bitcoin DeFi (Mobile) | wallets | 1-4 | 2-5 | 6-10 | supported |
| 159 | Claymorphism (Mobile) | Children education apps | 6-10 | 5-9 | 1-5 | supported |
| 160 | Claymorphism (Mobile) | teen social products | 6-10 | 5-9 | 1-5 | supported |
| 161 | Enterprise SaaS (Mobile) | B2B backend management | 1-4 | 2-5 | 6-10 | supported |
| 162 | Enterprise SaaS (Mobile) | productivity tools | 1-4 | 2-5 | 6-10 | supported |
| 163 | Sketch Hand-Drawn (Mobile) | Low-fidelity prototyping | 6-10 | 5-9 | 1-5 | supported |
| 164 | Sketch Hand-Drawn (Mobile) | creative brands | 6-10 | 5-9 | 1-5 | supported |
| 165 | Neumorphism (Mobile) | Minimal hardware controls | 6-10 | 5-9 | 1-5 | not-recommended |
| 166 | Neumorphism (Mobile) | smart home apps | 6-10 | 5-9 | 1-5 | not-recommended |
| 167 | Fluent 2 | Microsoft 365 | 1-4 | 2-5 | 6-10 | supported |
| 168 | Fluent 2 | Windows | 1-4 | 2-5 | 6-10 | supported |
| 169 | Shopify Polaris | Shopify admin apps | 2-5 | 3-5 | 4-7 | supported |
| 170 | Shopify Polaris | merchant tools | 2-5 | 3-5 | 4-7 | supported |
| 171 | Adobe Spectrum | Creative tools | 1-4 | 2-5 | 6-10 | supported |
| 172 | Adobe Spectrum | media workflows | 1-4 | 2-5 | 6-10 | supported |
| 173 | Spectrum 2 | New Adobe-style creative and document surfaces adopting Spectrum 2 | 6-10 | 5-9 | 1-5 | supported |
