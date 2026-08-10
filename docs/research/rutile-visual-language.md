# Rutile Visual Language — Research for the Rutile Design System

> **Status: Historical design-research snapshot (2026-07-10).** Source availability and external references were not refreshed; `DESIGN-SYSTEM.md` records the implemented direction.

Research seed for the "Rutile" design-language interview. Rutile is the working brand for the
Rutile markdown editor; KyaniteLabs is the umbrella studio. Three lanes: the mineral, the
ceramic glaze, and kyanite as the studio-level counterpart.

**Sourcing notes.** mindat.org's mineral pages sit behind bot protection and could not be fetched
directly in this environment; where mindat is referenced it is via its photo-gallery URLs (which
surfaced through search) or via secondary databases (webmineral.com, gemdat.org — the latter is
mindat's sister gemology database). digitalfire.com and glazy.org were fetched directly. All
palette hex/OKLCH values in this document are **derived interpretations by the researcher** —
sampled/estimated from the written specimen and glaze descriptions cited next to each palette,
not measured from calibrated photos.

---

## Lane 1 — Rutile the mineral

### Crystal habit: needles, prisms, elbow twins

- Rutile is tetragonal TiO2, the most common titanium dioxide mineral (polymorphs: anatase,
  brookite). Name from Latin *rutilus*, "reddish," after its dark-red lustrous crystals.
  Source: https://www.minerals.net/mineral/rutile.aspx
- Habit: "long and slender, straight prismatic crystals, often deeply striated and with steep
  complex terminations... Often in capillary needles and dense reticulated forms, in acicular
  habit, in delicate snowflake-like aggregates, and in star-shaped formations of dense needle
  groupings." Source: https://www.minerals.net/mineral/rutile.aspx
- Twinning is very common and characteristic: "sixlings, eightlings (both in the form of endemic
  rutile twins), knee-shaped twins, and v-shaped twins."
  Source: https://www.minerals.net/mineral/rutile.aspx
  Gem literature adds "often geniculate twinned crystals" (geniculate = elbow/knee-bend) and
  "frequently form as elbow or cyclic twins." Source: https://www.gemdat.org/gem-3486.html
- Prisms have square cross-section, striated parallel to the long axis.
  Source: https://skyjems.ca/pages/encyclopedia-rutile
- webmineral habit summary: acicular (needle-like), prismatic, massive-granular.
  Source: http://webmineral.com/data/Rutile.shtml

Design-usable habit vocabulary: **needle, capillary, striation, reticulated net, star/sunburst
spray, elbow (geniculated) bend, cyclic sixling/eightling wheel**.

### Color range and luster

- minerals.net color list: "Dark-red, metallic-gray, brownish-red, orange-red, reddish-black,
  golden-yellow, straw yellow"; luster "adamantine, submetallic." Even the opaque metallic forms
  are "somewhat translucent on edge under backlighting, with a dark red translucent tinge."
  Source: https://www.minerals.net/mineral/rutile.aspx
- webmineral color list: "Blood red, Bluish, Brownish yellow, Brown red, Violet"; luster
  adamantine; streak grayish black; reflected-light color "gray with bluish tint."
  Source: http://webmineral.com/data/Rutile.shtml
- Iron and other transition-metal substitution drives the reddish-brown to black coloration;
  Nb/Ta substitutions deepen the black. Pure rutile is colorless to pale yellow.
  Source: https://skyjems.ca/pages/encyclopedia-rutile
- Gem-reference colors: "(Dark) yellowish brown, reddish brown, black"; luster
  "Adamantine, Metallic." Source: https://www.gemdat.org/gem-3486.html

### Rutilated quartz ("Venus hair stone," flèches d'amour)

- "Rutile is well known for its habit of forming needle-like inclusions within other minerals,
  especially Quartz, in the form of long and slender yellow straw-like crystals... from scattered
  needles to dense parallel fibers." Source: https://www.minerals.net/mineral/rutile.aspx
- Variety names on the same page: **Venus Hairstone** ("capillary or dense acicular, hairlike
  sprays of Rutile") and **Sagenite** (acicular/reticulated form).
  Source: https://www.minerals.net/mineral/rutile.aspx
  ("Flèches d'amour" is the traditional French trade name for the same material; I did not find
  it on a primary source in this session — treat as unverified-but-conventional.)
- "Rutilated Quartz is one of the few gemstones desirable *because* of its inclusions... exists
  mainly in colorless, transparent Quartz, although a brown, smoky Rutilated Quartz also exists."
  Source: https://www.minerals.net/gemstone/rutilated_quartz_gemstone.aspx
- Rutile inclusions also cause asterism/chatoyancy (star sapphire silk); dense intersecting rutile
  networks produce the six-rayed star. Sources: https://www.minerals.net/mineral/rutile.aspx ,
  https://skyjems.ca/pages/encyclopedia-rutile
- The classic locality is Novo Horizonte (Remédios), Bahia, Brazil — golden needle sprays,
  often epitaxially radiating off mirror-black hematite plates ("rutile stars").
  Sources: https://www.minerals.net/mineral/rutile.aspx , http://webmineral.com/data/Rutile.shtml

**Reference specimen images (lane 1):**

1. Golden rutile needles epitaxial on black hematite, Novo Horizonte —
   http://webmineral.com/data/Rutile.shtml (Fabre Minerals photo on page)
2. Rutilated quartz thumbnail with hematite, Novo Horizonte —
   https://www.irocks.com/minerals/specimen/53617
3. Transparent quartz crystal full of slender golden rutile needles on hematite pedestal —
   https://www.irocks.com/minerals/specimen/51006
4. Mindat photo gallery, rutilated quartz localities, Remédios/Novo Horizonte (polished sections,
   golden needles in water-clear quartz) — https://www.mindat.org/gallery.php?loc=412900&pco=1
5. Rutile-hematite six-armed star inside water-clear quartz —
   https://www.mcdougallminerals.com/product/quartz-rutile-hematite/
6. Golden rutile starburst spray >5 cm on quartz —
   https://fineartminerals.com/rutile/rutile-hematite-quartz-brazil/
7. Rutilated quartz (rough) — https://s3-us-west-2.amazonaws.com/reference/images/materials/quartz_rutilated.jpg
   (from https://digitalfire.com/material/rutile)
8. minerals.net photo set (golden dense, reddish-brown, thinly rutilated variants) —
   https://www.minerals.net/gemstone/rutilated_quartz_gemstone.aspx

### Optics: why cut rutile out-fires diamond

| Property | Value | Source |
|---|---|---|
| Refractive index | uniaxial (+), ω = 2.605–2.621, ε = 2.899–2.908 | http://webmineral.com/data/Rutile.shtml |
| RI (gem tables) | 2.609–2.903 | https://www.gemdat.org/gem-3486.html |
| Birefringence | 0.287–0.294 (webmineral); 0.287 (gem tables) — among the highest of common minerals; visible facet-edge doubling | http://webmineral.com/data/Rutile.shtml , https://www.gemdat.org/gem-3486.html |
| Dispersion | 0.280 vs diamond's 0.044 (~6–7× diamond); gemdat rates it "very high" | https://skyjems.ca/pages/encyclopedia-rutile , https://www.gemdat.org/gem-3486.html |
| Luster | adamantine to metallic | https://www.gemdat.org/gem-3486.html |
| Hardness / SG | 6–6.5 Mohs / 4.2–4.3 | https://www.minerals.net/mineral/rutile.aspx |

- Synthetic rutile ("Titania") was a 1940s–50s diamond simulant precisely because its fire
  *exceeds* diamond's — so much so the effect was "often overwhelming rather than convincing."
  Source: https://skyjems.ca/pages/encyclopedia-rutile
- Curiosity with brand potential: pale synthetic rutile turns **blue** when heated >1000 °C in a
  reducing atmosphere (reversible in oxygen) — a mineral echo of the reduction "rutile blue"
  glaze story in Lane 2. Source: https://www.gemdat.org/gem-3486.html (citing Nassau 1984)

### Derived palette candidates (lane 1) — interpretations, not measurements

**Palette R1 — "Golden Needle on Smoky Ground"** (derived from: golden-needle-in-smoky/clear
quartz specimens — iRocks 51006, mindat gallery loc=412900, minerals.net "Golden Dense Rutilated
Quartz" photo set)

| Token idea | Hex | OKLCH | Derived from |
|---|---|---|---|
| smoke-950 (ground) | `#241E19` | `oklch(0.240 0.013 61.9)` | smoky quartz body tone |
| smoke-800 | `#3A322A` | `oklch(0.323 0.018 67.1)` | smoky quartz mid |
| quartz-mist | `#E9E2D6` | `oklch(0.915 0.018 81.3)` | milky/clear quartz highlight |
| rutile-gold (accent) | `#C9921E` | `oklch(0.696 0.136 79.9)` | golden needle body color |
| rutile-glint | `#F0C24B` | `oklch(0.834 0.143 87.3)` | lit needle / adamantine glint |
| hematite-ink | `#1C1A1B` | `oklch(0.220 0.004 345.6)` | hematite plate the needles radiate from |

**Palette R2 — "Blood-Red Prism"** (derived from: webmineral "blood red" color entry; minerals.net
"dark red and etched prismatic crystals... Hiddenite, North Carolina"; dark-red-translucent-on-edge
backlighting note)

| Token idea | Hex | OKLCH | Derived from |
|---|---|---|---|
| prism-red-900 | `#2E1512` | `oklch(0.231 0.042 27.9)` | reddish-black opaque crystal body |
| prism-red-700 | `#7A1F14` | `oklch(0.387 0.127 30.5)` | blood-red transmitted-light core |
| prism-red-500 | `#A03A22` | `oklch(0.489 0.141 34.5)` | orange-red edge translucence |
| copper-warm | `#C97C4A` | `oklch(0.658 0.116 52.4)` | brownish-red / copper needle variant |
| adamantine-high | `#E8B891` | `oklch(0.816 0.076 60.5)` | specular highlight on striated face |

**Palette R3 — "Iron Rutile, Metallic"** (derived from: minerals.net "mirror-like metallic-looking
crystals" (Graves Mountain, GA); webmineral reflected-light color "gray with bluish tint," streak
grayish black; red backlit tinge)

| Token idea | Hex | OKLCH | Derived from |
|---|---|---|---|
| iron-black | `#17161A` | `oklch(0.203 0.008 297.1)` | iron-rich black rutile body |
| gunmetal-600 | `#4A4E58` | `oklch(0.424 0.017 268.3)` | submetallic gray, bluish RL tint |
| gunmetal-400 | `#8A8F9A` | `oklch(0.649 0.017 266.2)` | brushed metallic mid |
| specular-200 | `#B9BDC6` | `oklch(0.798 0.013 266.7)` | mirror-face highlight |
| ember-edge | `#5E2A20` | `oklch(0.355 0.079 32.4)` | dark-red translucent edge under backlight |

---

## Lane 2 — Rutile in ceramic glaze chemistry

Authorities used: digitalfire.com (Tony Hansen's Digitalfire Reference Library) and glazy.org.

### What ceramic rutile is and what it does

- Ceramic "rutile" is the impure ground mineral: ~90% TiO2 with ~10% Fe2O3 in the generic
  analysis; industry accepts up to 15% contaminants (below 85% TiO2 it's called ilmenite).
  Sold as light (calcined) tan powder, darker uncalcined powder, or granular.
  Source: https://digitalfire.com/material/rutile
- "Rutile produces many crystalline, speckling, streaking, and mottling effects in glazes during
  cooling in the kiln... many attractive variegated glazes are made using it. Many potters would
  say that their living depends on their rutile supply!"
  Source: https://digitalfire.com/material/rutile
- It is "more often considered a variegator than a colorant." It encourages micro-crystal growth
  (it is crystalline itself) and rivulets, especially in high-melt-fluidity (high-boron) bases.
  Source: https://digitalfire.com/material/rutile
- **Not the same as pure TiO2:** "you can[not] use a 95% titanium:5% iron mix and get the same
  result... The mineralogy and significant other impurities in rutile are a major factor," and
  coarser particle size matters too. Source: https://digitalfire.com/material/rutile
  (Counterpoint on the same site: for the specific floating-blue effect, TiO2 + an iron-bearing
  base can reproduce it and is *more consistent* — the GA6-C vs L4655 comparison.
  Source: https://digitalfire.com/material/rutile , "Titanium instead of rutile for floating blue")

### Percentages, iron/boron interaction, batch variability

- "As little as 2% can impart significant effects in stoneware glazes... In glazes with high melt
  fluidity (e.g. having high boron), large amounts of rutile (e.g. 6-8%) can be quite stunning."
  Typical additions land in the 2–8% band; 4% is the classic floating-blue amount; "the addition
  of 4-8% rutile to many stoneware glazes can make an otherwise drab or flat glaze become much
  more interesting." Source: https://digitalfire.com/material/rutile
- Cliff-edge behavior: at 5%+ "a given percentage might work well whereas a slightly higher
  amount can look drastically different" — and excess turns "an ugly yellow in a mass of titanium
  crystals." Source: https://digitalfire.com/material/rutile
- Batch-to-batch variability is structural: natural ores blended from different world deposits,
  contaminant swings, and grind fineness all shift results; "large users of rutile will often
  track batch numbers and test when the number changes." Milling fineness alone can make or break
  the variegated blue effect. Source: https://digitalfire.com/material/rutile
- **Rutile blue / floating blue mechanism:** "Rutile blue glazes are actually titanium blues...
  The iron and titanium in the rutile react to form the floating blue effect." In reduction,
  "5-8% added rutile can give powder to deep blue colors... brilliant and mottled with shades of
  browns and tans," dependent on adequate silica (≈7× alumina or more) and on the iron content —
  which is why the blue drifts when the rutile supply changes.
  Source: https://digitalfire.com/material/rutile
- The classic cone 6 **Floating Blue** (David Shaner, popularized by James Chappell): Nepheline
  Syenite 47.9 / Gerstley Borate 27 / Silica 20.3 / EPK 5.5 + iron oxide 2, cobalt oxide 1,
  **rutile 4**. Its variegation stack: thickness-dependent color, phase separation ("swirl" of
  rivulets), titanium micro-crystal sparkle, opalescent calcium-borate boron-blue crystals,
  bubble pooling, iron speckle. Famously "fickle."
  Source: https://digitalfire.com/recipe/g2826r
- Edge-break behavior: melt-fluid rutile blues thin "on the edges of contours and break to the
  color of the underlying body. It looks best on dark bodies."
  Source: https://digitalfire.com/recipe/g2826r ("Alberta Slip blue" GA6-C1 caption)
- Tan/cream side of rutile: "many shades from pale straw to tan to cinnamon brown to orange
  brown"; "soft tan colors due to its iron content, especially in matte glazes"; orange-tan in
  zinc glazes; oatmeal mattes (GA6-F "Alberta Slip Cone 6 Oatmeal" exists as a named recipe).
  Sources: https://digitalfire.com/material/rutile , https://digitalfire.com/recipe/g2826r
- Community-verified oxidation rutile blue on glazy: 4% rutile over a cone 6-7 base
  (SiO2:Al2O3 = 9.3 — note it satisfies the high-silica condition), best on iron-rich bodies,
  "falls flat/white on low iron clay bodies," color shifts with thickness and SG, thin
  application goes purple. Source: https://glazy.org/recipes/113219

### The visual vocabulary (glaze lane)

**Streaked, mottled, variegated, rivulets, crystalline bloom (micro-crystals that "sparkle"),
opalescence (boron-blue), floating (color suspended in a translucent depth), edge-break
(thin edges revealing the body underneath), speckle, oatmeal.** All terms as used on
https://digitalfire.com/material/rutile and https://digitalfire.com/recipe/g2826r ;
digitalfire's glossary pages "Variegation" and "Reactive Glazes" are linked from the rutile page.

**Reference images of fired rutile glazes (lane 2):**

1. Rutile blue mug photos (3 shots incl. thin-application purple shift, iron-wash variant) —
   https://ddms6z64wp3a6.cloudfront.net/uploads/recipes/19/m_113219.5fe4b0a45c98c.jpg ,
   https://ddms6z64wp3a6.cloudfront.net/uploads/recipes/19/m_113219.60cf1e1d3278c.jpg ,
   https://ddms6z64wp3a6.cloudfront.net/uploads/recipes/19/m_113219.655e0bbc3ff56.jpg
   (all from https://glazy.org/recipes/113219)
2. H23 Rutile Blue tile — https://ddms6z64wp3a6.cloudfront.net/uploads/recipes/48/s_301548.63e8b324143e4.jpg
   (from https://glazy.org/recipes/301548, surfaced on the 113219 page)
3. GA6-C rutile blue on brown stoneware (2/3/4/5% rutile line blend "the rutile mechanism") —
   https://s3-us-west-2.amazonaws.com/reference/images/g2862-431x170-6.jpg
   (from https://digitalfire.com/material/rutile)
4. GA6-C rutile blue vs ball-milled granular rutile comparison —
   https://s3-us-west-2.amazonaws.com/reference/images/pictures/crmhqxbbr7.jpg
   (from https://digitalfire.com/material/rutile)
5. "The Magic of Rutile Glazes" — blue/navy rivulets and crystals, slow-cooled cone 6 —
   https://s3-us-west-2.amazonaws.com/reference/images/pictures/hfqcnzqyal.jpg
   (from https://digitalfire.com/material/rutile)
6. Floating Blue close-up on dark/buff bodies —
   https://s3-us-west-2.amazonaws.com/reference/images/pictures/smtywgg6ik.jpg
   (from https://digitalfire.com/recipe/g2826r)
7. GR6-M Ravenscrag Floating Blue showcase piece —
   https://s3-us-west-2.amazonaws.com/reference/images/recipes/laphykajyx.jpg
   (from https://digitalfire.com/recipe/g2826r)
8. Floating blue over black engobe (edge-break demo) —
   https://s3-us-west-2.amazonaws.com/reference/images/pictures/kw8dpqix8o.jpg
   (from https://digitalfire.com/recipe/g2826r)

### Derived palette candidates (lane 2) — interpretations, not measurements

**Palette G1 — "Rutile Blue / Floating Blue"** (derived from: glazy 113219 photo set and
digitalfire GA6-C / G2826R imagery — variegated slate blue floating over tan breaks on
iron-rich body)

| Token idea | Hex | OKLCH | Derived from |
|---|---|---|---|
| float-navy | `#2B3D57` | `oklch(0.357 0.051 257.5)` | deep navy pooling in thick areas |
| float-slate | `#4C6B8A` | `oklch(0.517 0.061 249.1)` | main variegated slate blue |
| thin-violet | `#6B6B94` | `oklch(0.544 0.064 284.1)` | thin-application purple hue (glazy 113219 note) |
| break-tan | `#B08D57` | `oklch(0.664 0.083 77.4)` | tan edge-break to body color |
| cream-pool | `#D8CBB2` | `oklch(0.846 0.037 83.8)` | lighter pooled/opalescent areas |
| body-umber | `#6E4F2F` | `oklch(0.453 0.063 66.0)` | iron-rich clay body ground |

**Palette G2 — "Oatmeal / Tan Break"** (derived from: digitalfire's rutile color mechanisms —
"pale straw to tan to cinnamon brown to orange brown," soft tan mattes, oatmeal recipe family)

| Token idea | Hex | OKLCH | Derived from |
|---|---|---|---|
| oatmeal | `#CFC3AB` | `oklch(0.821 0.035 84.6)` | oatmeal matte field |
| straw | `#D9B36C` | `oklch(0.785 0.100 82.1)` | pale straw |
| tan | `#A98C5F` | `oklch(0.656 0.071 78.0)` | classic rutile tan |
| cinnamon | `#8A5A33` | `oklch(0.512 0.083 58.5)` | cinnamon brown break |
| glaze-white | `#F0E8D8` | `oklch(0.933 0.023 84.6)` | unbroken cream surface |

---

## Lane 3 — Kyanite, the studio umbrella

### The mineral facts

- Kyanite (Al2SiO5, triclinic; polymorphs andalusite and sillimanite) — "one of the most
  attractive blue minerals in nature... intense shades of blue, or even multiple shades with
  color zoning in a single crystal." Named from Greek *cyanos*, deep blue.
  Source: https://www.minerals.net/mineral/kyanite.aspx
- **Bladed habit:** "Most often in long and slender bladed crystals. Also in bladed crystal
  groups, radiating, in veins, reticulated, and in flattened tabular crystals."
  Source: https://www.minerals.net/mineral/kyanite.aspx
- **Two hardnesses (anisotropic hardness):** "Kyanite is strongly anisotropic... the most
  well-known anisotropic mineral. The vertical hardness of Kyanite ranges from 4.5 to 5.5, and
  horizontal hardness from 6 to 7." Source: https://www.minerals.net/mineral/kyanite.aspx
  Gem tables state it as "4 – 4.5 along axes; 6 – 7 across axes."
  Source: https://www.gemdat.org/gem-2303.html
  (The exact low-end figure varies by source, 4–5.5 along the blade; the two-hardness fact
  itself is solid.)
- **Color zoning:** "A frequent habit is a deeper colored blue streak running through the center
  of a crystal," often "multicolored with different shades of blue or white."
  Source: https://www.minerals.net/mineral/kyanite.aspx — i.e., darker blue cores with pale/white
  margins is the canonical look (see the Vitória da Conquista "blue vein in the center" locality
  note on the same page).
- **Pleochroism:** blue stones show "strong trichroism: colorless/pale blue –
  (greenish or violet)-blue – dark blue." Source: https://www.gemdat.org/gem-2303.html
  (webmineral gives X colorless, Y colorless/pale blue: http://webmineral.com/data/Kyanite.shtml)
- Optics for contrast with rutile: RI 1.710–1.735, birefringence 0.012–0.033, dispersion 0.020,
  vitreous-to-pearly luster. Sources: https://www.gemdat.org/gem-2303.html ,
  http://webmineral.com/data/Kyanite.shtml
- Blue is caused chiefly by Fe2+–O–Ti4+ charge transfer (plus Fe–Fe transfer and Cr) — kyanite's
  blue literally requires **titanium**, rutile's element. Source: https://www.gemdat.org/gem-2303.html
- Environment: metamorphosed schists and gneisses (also granite pegmatites).
  Sources: https://www.minerals.net/mineral/kyanite.aspx , http://webmineral.com/data/Kyanite.shtml

### Does the mineralogy support the Rutile-inside-KyaniteLabs pairing? Yes — strongly.

Verified co-occurrence facts:

1. **Rutile and kyanite are a standard couple in eclogites** (the signature high-pressure
   metamorphic rock): "Rutile, kyanite, and quartz are typically present" in eclogite.
   Source: https://www.alexstrekeisen.it/english/meta/eclogite.php (University-adjacent petrology
   teaching site, Alex Strekeisen)
2. **They occur locked together as inclusion assemblages:** an eclogite-facies association of
   "phengite + kyanite + rutile" preserved as inclusions in garnet, Lewisian Gneiss Complex,
   NW Scotland. Source: https://pubs.geoscienceworld.org/gsa/geology/article/54/3/269/723875/Eclogite-facies-metamorphism-of-continental-crust
3. **Rutile needles occur *inside* kyanite** — literally the product inside the studio mineral:
   "oriented rutile needles in kyanite matrix in grospydite LUV134/10 (Udachnaya)."
   Source: https://www.researchgate.net/figure/Inclusions-in-kyanite-A-and-garnet-B-C-in-studied-eclogites-A-oriented-rutile_fig6_272028077
   Also: rutile as monomineralic inclusions in garnet, **kyanite**, quartz and zircon in the
   diamondiferous kyanite gneisses of the Kokchetav massif.
   Source: https://www.sciencedirect.com/science/article/pii/S0024493721002085
4. The pair is so routine that titanite + kyanite = anorthite + **rutile** is a published
   geobarometer for eclogites. Source: https://www.usgs.gov/publications/reaction-titanitekyaniteanorthiterutile-and-titanite-rutile-barometry-eclogites
5. Chemical kinship: kyanite's blue is an Fe–**Ti** charge-transfer color
   (https://www.gemdat.org/gem-2303.html), and rutile *is* the titanium mineral. The studio
   color is caused by the product's element.

Honest assessment of the design thesis:

- **Warm/cool complementarity: real.** Rutile's canon is golden-yellow through blood-red
  (oklch hues ~30–90); kyanite's is blue (~250–270). These are near-complementary in OKLCH hue
  space, and both sit against the same neutral grounds (quartz white/smoke, gray gneiss).
- **Needle vs blade: real and clean.** Rutile = thin acicular needles, capillary hair, radiating
  stars (line-weight: hairline). Kyanite = flat bladed crystals, like knife/plank forms
  (line-weight: broad flat strokes). Same "elongated crystal" family, opposite stroke widths —
  a legitimate motif duality (hairline rules/accents for Rutile; broad flat bars/blades for
  KyaniteLabs).
- **Optical contrast is a bonus axis:** rutile is the fire mineral (dispersion 0.280, adamantine,
  RI 2.6+); kyanite is quiet (dispersion 0.020, vitreous-pearly, RI 1.71). Loud sparkle inside a
  calm studio.
- **One asymmetry to embrace, not hide:** in the rocks, rutile is the small accessory phase inside
  big kyanite-bearing systems — which matches the brand architecture (small sharp product inside
  the studio) rather than undermining it.
- **The gift line, verified:** *rutile needles are found inside kyanite crystals* (Udachnaya
  grospydite, Kokchetav gneiss). "KyaniteLabs products are the golden needles inside the blue
  blade" is geologically true.

### Derived palette candidate (lane 3) — interpretation, not measurement

**Palette K1 — "Kyanite Blade"** (derived from: minerals.net Barra do Salinas deep-blue-in-white-
quartz photos and the "darker blue vein in the center, pale edges" zoning description;
webmineral Barra do Salinas specimen photo caption "bright blue crystals in white quartz matrix")

| Token idea | Hex | OKLCH | Derived from |
|---|---|---|---|
| kyanite-core | `#23386B` | `oklch(0.352 0.093 265.2)` | dark blue central vein/core zone |
| kyanite-blade | `#2E4A80` | `oklch(0.415 0.097 262.3)` | main blade blue |
| kyanite-flash | `#5B7FBE` | `oklch(0.597 0.105 261.1)` | pleochroic violet-blue flash |
| kyanite-zoned | `#96A9C9` | `oklch(0.731 0.051 260.8)` | pale zoned margin |
| blade-edge | `#C7D3E8` | `oklch(0.864 0.032 261.7)` | near-white blade edge |
| quartz-matrix | `#EFEDE8` | `oklch(0.946 0.007 88.6)` | white quartz matrix ground |

---

## Texture & motif vocabulary

| Motif | Mineral/glaze source | Design translation candidates |
|---|---|---|
| **Needle / acicular** | rutile habit (minerals.net, webmineral) | hairline rules, 1px accents, cursor/caret identity, list markers |
| **Inclusion** | golden needles *inside* clear/smoky quartz; rutile inside kyanite | content framed within a calm translucent container; accents embedded in neutral surfaces, never floating outside them |
| **Striation** | lengthwise striations on rutile prisms (gemdat, minerals.net) | fine parallel line textures, subtle vertical rhythm |
| **Geniculated (elbow) twin** | rutile twinning (gemdat, minerals.net) | a signature bent-line mark; angle-break motif in logo/dividers |
| **Sixling / cyclic twin** | rutile eightlings/sixlings (minerals.net) | radial star/asterisk mark; the md `*` glyph as brand pun |
| **Variegation / mottling / streaking** | rutile glazes (digitalfire) | organic texture language for generated/"chance" surfaces; noise fields; non-uniform backgrounds |
| **Rivulet / floating** | floating blue mechanism (digitalfire g2826r) | color that reads as suspended in depth — layered translucency, soft inner glows |
| **Crystalline bloom / sparkle** | titanium micro-crystals (digitalfire) | micro-interaction glints, sparse highlight particles |
| **Edge-break** | thin glaze breaking to body color on contours (digitalfire) | edges/borders that reveal the underlying ground color; hover states that "thin" the surface |
| **Luster: adamantine vs vitreous** | rutile adamantine/metallic vs kyanite vitreous-pearly (gemdat) | product accents get high-chroma "fire"; studio surfaces stay low-chroma satin |
| **Blade** | kyanite bladed habit (minerals.net) | broad flat bars, wide brush strokes, studio-level headers/wordmark |
| **Zoning** | dark core / pale edge kyanite zoning (minerals.net) | gradient bars darker at center; focus states with concentrated core color |
| **Speckle / oatmeal** | granular rutile speck, oatmeal mattes (digitalfire) | paper-grain neutrals for reading surfaces |

---

## Design-direction hypotheses for the tastecheck interview

1. **Golden-needle accent system on a smoky neutral ground.** Palette R1 as the core app theme:
   warm near-black smoky ground (`#241E19`-family), quartz-mist text surfaces, and a single
   high-chroma rutile-gold accent (`#C9921E`/`#F0C24B`) used at hairline weight only — carets,
   active borders, link underlines, selection needles. The accent is always an *inclusion*:
   thin, embedded, never a filled slab. (Dark-first; light theme inverts to clear-quartz white
   with the same gold needles.)

2. **Variegation as the texture language for chance/ambient states.** Rutile glazes are prized
   because no two firings match (digitalfire's batch-variability story). Use subtle variegated/
   mottled fields (Palette G1/G2) for the non-deterministic surfaces of the editor — empty
   states, splash, preview backgrounds, sync/"firing" progress — while keeping the writing
   surface strictly calm. The batch story ("same recipe, different firing") is also honest
   language for markdown → render.

3. **Kyanite blue reserved for studio-identity moments only.** Palette K1 appears solely where
   KyaniteLabs speaks: about screens, licensing, footer wordmark, installer/first-run. Inside
   the product it never competes with rutile gold — mirroring the mineralogy (kyanite is the
   host rock; rutile is the accessory that catches the light). Needle (hairline, gold) vs blade
   (broad flat bar, blue) becomes the grammar separating product UI from studio branding.

4. **Edge-break as the interaction metaphor.** Borrow the glaze behavior where melt-fluid rutile
   blue thins over edges and breaks to the body color: hover/focus/active states "thin" a
   surface to reveal the warmer ground beneath (e.g., a card's border warms to break-tan on
   hover). Gives a physical, non-generic state system with cited precedent
   (https://digitalfire.com/recipe/g2826r).

5. **Fire budget: one dispersive moment per screen.** Rutile's dispersion (0.280 vs diamond
   0.044) failed as a diamond simulant because the fire was *overwhelming* — a great taste
   lesson. Encode it: at most one prismatic/high-chroma "fire" element per screen (a glint on
   save, the geniculated-twin logo mark, a star-twin easter egg), everything else adamantine-dark
   or oatmeal-quiet. The restraint rule is itself the brand story.

---

*Researched 2026-07-10. Fetch limitations this session: firecrawl and exa quotas were exhausted;
mindat.org page bodies are behind bot protection (its gallery URLs and sister site gemdat.org
were used instead); GIA's site was not reached directly. Palette values are researcher-derived
interpretations of cited specimen/glaze descriptions and photos, intended as starting points for
the design-system interview, not measured colors.*
