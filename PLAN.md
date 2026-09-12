# apricorn — genomförandeplan

Reviderad **2026-09-12** efter kodgranskning och research. Vi utvecklar i små,
verifierbara delar. Aktuell prioritet är rendererparitet i **5B**, med den
baseline och de mätkontrakt från **5A** som verifieringen behöver.

- [Omvärdering, belägg och externa källor](docs/planning/review-2026-09-12.md).
- [Detaljspecifikation för närmaste arbetskort](docs/planning/next-slices.md).
- [Tidigare plan, bevarad oförändrad](docs/planning/plan-before-review-2026-09-12.md).

## Mål och avgränsning

Beteendemässigt likvärdig Rustimplementation av **Pokémon HeartGold US**, som
läser användarens original-ROM vid körning. ROM SHA-1:
`4fcded0e2713dc03929845de631d0932ea2b5a37`. Målplattformar: Windows, Linux,
macOS och Android. Samma maskininstruktioner krävs inte.

1:1 betyder, för lika initialtillstånd och externa input:

- Samma spelbeslut, integeraritmetik, RNG-värden och ordning/antal RNG-drag.
- Samma persistenta tillstånd, retail saveformat och observerbara savebeteende.
- Samma input-, händelse-, text-, rörelse- och scenordning på definierad tidsaxel.
- Samma logiska bildinnehåll och verifierad rendering vid DS-upplösning, även
  under rörelse. Pixellikhet anges bara för faktiskt verifierad täckning.
- Samma ljudhändelser och uppspelningsbeteende; sampleexakthet anges separat.

**Kampanjparitet** (Johto → credits → Kanto → Red) är en delmilstolpe.
**Full originalomfattning** kräver även 8C. Trådlöst, IR, Pokéwalker, mikrofon
eller externa distributionsfunktioner får inte tyst undantas. SoulSilver,
andra språk/ROM-revisioner och QoL/modding ingår inte i första målprofilen.

ROM, extraherade assets, saves och råa bilder/state/PCM hålls lokalt.
Speldata läses ur ROM; inga manuellt kopierade dialoger/tabeller.
Proveniens/licens dokumenteras före kodåterbruk. melonDS behålls som separat
orakel; ingen kopiering eller översättning av GPL-kod till MIT-kärnan.
Se research R1–R7 för referensernas användningsgränser.

## Arbetssätt och avbockning

### Gren och PR för varje ändring

1. Kontrollera aktuell arbetsmapp, gren, worktrees och remotes innan ändring.
   Utveckla på en namngiven `codex/`-gren, aldrig direkt på `main`.
2. Checka ut arbetsgrenen i den arbetsmapp där arbetet faktiskt sker. Om en
   separat worktree behövs, ange dess absoluta sökväg och gren tydligt; en gren
   i en annan worktree betyder inte att den aktuella arbetsmappen bytt gren.
3. Bevara befintliga lokala ändringar. Commita endast den överenskomna delen;
   dokumentation och pågående rendererarbete får separata, tydliga leveranser.
4. Pusha arbetsgrenen med upstream och verifiera att den finns på rätt remote.
   En lokal gren eller en worktree räcker inte som bevis på publicerad gren.
5. **PR-målet för detta projekt är `matte250/apricorn:main` (`upstream`).**
   `origin` pekar på `indateftww/apricorn`, en fork som får användas för
   push/head-grenen. Skapa då en PR från `indateftww:<arbetsgren>` till
   `matte250/apricorn:main`, inte till forkens `main`.
6. Verifiera PR:ens base-repo, base-gren, head-repo, head-gren och ändrade filer
   i GitHubs svar. Rapportera fullständig PR-länk och pushad gren. Enbart texten
   `main` räcker inte när flera remotes finns. Merga inte utan separat instruktion.

Aktuellt rendererarbete prioriterar **5B** enligt användarens instruktion.
Mät-/baselineuppgifter från 5A genomförs i den omfattning som rendererjämförelsen
behöver. Klippning/culling, normals/ljus/toon/shininess, Z/W-buffer, skuggor och
fog/edge/AA är öppna verifieringskrav även där implementation redan finns.
Field-goldens bevisar determinism; full pixelparitet kräver oraclejämförelser.

### Små leveranser och acceptans

En lövuppgift är en liten, sammanhängande leverans som kan granskas och
återställas självständigt. Flera oberoende algoritmer eller funktioner innebär
att uppgiften först delas till exempelvis `6C.05a`, `6C.05b`. Senare regioner
eller mekanikfamiljer är planeringspaket, aldrig en enda implementationsändring.

| Status | Betydelse |
| --- | --- |
| Planerad | `[ ]`, kontraktet är inte verifierat färdigt. |
| Implementerad | Kod finns; fortfarande `[ ]` om integration/bevis saknas. |
| Verifierad | `[x]`, kontrakt uppfyllt, integrerat och bevis länkat. |
| Accepterad delmilstolpe | Automatiska bevis och dokumenterad manuell regression för spelbar/visuell del. |
| Blockerad | `[ ]`, konkret beroende/kunskapslucka och nästa mätuppgift angivna. |

Arbetskort ska före implementation ange ID, användarresultat, beroenden,
moduler, originalreferens, input/output, state, tids-/RNG-/savekontrakt,
fel/gränsfall och acceptans. De närmaste korten finns i detaljspecifikationen.
Okända binärkontrakt mäts först; en lång checklista ersätter inte kunskap.

Gemensamt klartvillkor:

1. Riktig konsument används vid anspråk på spelbarhet. `RecordingHost` bevisar
   bara VM-/hostkontrakt, inte en fungerande scen.
2. Berörda tester passerar. Körda ROM-/oracletester och överhoppade tester
   redovisas separat, med kommandon/revisioner/resultat i arbetskortet.
3. Timing/RNG verifieras mot originalet. Motorintern renderhash visar
   regression/determinism, inte originalparitet.
4. Relevant negativt fall och gränsfall täcks; inget tyst no-op för saknat stöd.
5. Oraclebaseline ändras med motivering/jämförelse. Motorfel blir inte nytt facit.
6. Varje begränsning får eget öppet ID; inget ospecificerat ”fidelity senare”.

Vi tar normalt ett kort i taget. Planen kräver inte parallella agenter eller
stora separata grenprojekt. Manuell acceptans samlas vid spelbara delmilstolpar.
Historisk fas 4-acceptans 2026-09-11 behålls. Nya fas-/G-kryss stängs först när
alla krav i deras deklarerade omfattning är uppfyllda.

## Nuläge och omvärdering

Granskningen gäller `HEAD b15334c` plus befintliga lokala ändringar. Inga
speltester eller prestandamätningar kördes på nytt för denna planrevision.

| Del | Finns | Återstående |
| --- | --- | --- |
| Fas 0–1 | Workspace, ROM/format, extraction/cache, roundtrip | CI, felprov, nya modeller/animationer. |
| Fas 2 | ARM-prober, melonDS, trace/diff/input, corpus, enginerunner | Tidsaxel, provenance och state utöver tre RNG-regioner. |
| Fas 3–4 | Raster/presentation/OAM/text, boot/menu/Oak/name, save/init | Full intro/title, scenparitet, auktoritativ live-save. |
| Field | Sovrum → hus → New Bark, höjd/kollision/kamera/warps, NPC-objekt | Autonomi, scripts/dialog, objektkollision, fler kartor/warps. |
| VM/menu | Handlergrund, recording host, separat startmeny | Live host, waits, fältintegration, bag/party. |
| Grafik | GX-kod/rawdiff/native sampling och inkopplat ljus | Exakthet, material/animation, konsekutiva scenjämförelser. |
| Fas 6–7 | ROM-tabeller och SDAT-container | Pokémonobjekt, encounters, battle, appar och uppspelning. |
| Fas 8–9 | Backlog och desktop-skal | Innehållstäckning, releasegates, övriga plattformar. |

Viktiga korrigeringar: CI flyttas fram; oracle-vs-oracle är inte engineparitet;
VBlank/spelsteg/bootseed/RTC behöver förenas; ljus är redan inkopplat; 3D används
redan i fältet. Full Johto före battle/storysystem är ett cirkulärt exitkrav.
Party/bag-data behövs före menyer, starter/follower/Pokégear behöver tidiga delar.
Belägg och exakt kodunderlag finns i researchdokumentet.

## Beroenden och arbetsordning

Fasnumren behålls för kodlänkar och är inte en strikt kalenderordning.

| Ordning | Del | Kräver | Delmilstolpe |
| --- | --- | --- | --- |
| 1 | 5A.01–07:s grunddelar | Befintlig grund | Baseline, test-, tids-, state- och savekontrakt; 5A.06c slutförs med scripts. |
| 2 | 5B | 5A.01, 04–05 | Verifierad fältgrafik i namngivna scener. |
| 3 | 5C | 5A:s grundkontrakt; relevant 5B för visuell acceptans | Interaktion/dialog/Mom; slutför 5A.06c. |
| 4 | 5D | 5C | Startmeny/undre LCD/Save/tidsljus. |
| 5 | 6A, 6B.01–04 | 5A, 5C | Riktig starter, mon/bag-data, första follower. |
| 6 | 5E.01–05, 6B.05–08, första 6C/6D-striden | Respektive mekanik | Route 29 och första avslutade battle. |
| 7 | Resterande 6, 7 och 5E | Kortens egna krav | Rival/fångst/menyer/ljud/mekanikbredd. |
| 8 | 8A–8C | Stabil field/battle/save/audio | En innehållsetapp i taget. |
| 9 | 9A–9C | CI redan i 5A | Paket på målplattformarna. |
| 10 | Fas 10 | Godkänd specificerad 1:1-profil | Valbara utökningar. |

En icke blockerande GX-utredning hindrar inte isolerat headless VM-arbete.
Visuella gates krävs före scenacceptans. Första ljudslicen kan starta när
fältets ljudhändelser finns. Plattformsexperiment får göras tidigt.
En G-markering är acceptans av en samlad delmängd, inte ett krav att all annan
utveckling väntar. 5C.01 kräver 5A.06a/b, men inte scriptobservationen 5A.06c
som levereras med 5C. Därmed skapas inget cirkulärt gateberoende.

## Fas 0–4 — bevarad grund och kompletteringar

### Fas 0 — scope/workspace

Historiskt levererat: ROM-verifiering, workspace och pret-inventering.

- [ ] **0R.01** Uppdatera C/asm-inventering per subsystem mot pinnad revision. Klart: reproducerbart kommando och daterad tabell för scripts/handlers/overlayappar.
- [ ] **0R.02** Definiera maskinprofil: ROM, boot/firmware, RTC, save och externa enheter. Klart: varje corpusfall väljer identifierbar profil; samordnas med 5A.05.

### Fas 1 — assetpipeline

Historiskt levererat: NDS/NitroFS, LZ/BLZ, NARC, NCGR/NCLR/NSCR, NANR/NCER,
BTX, meddelanden, SDAT-container, extract/cache/roundtrip. Parsers ligger
huvudsakligen i core; tools är CLI.

- [ ] **1R.01** Inventera använda formatvarianter/NSBMD/SBC/GX/animationer. Klart: varje nådd asset har stöd eller namngivet gap; inget tyst tomt lager.
- [ ] **1R.02** Prova trunkering/offsetoverflow/orimliga antal/dekompressionsgränser med syntetiska fall/fuzzmål. Klart: fel utan panik eller obegränsad allokering.
- [ ] **1R.03** Kontrollera parsersemantik oberoende av roundtrip och cacheinvalidering med ROM/verktygs/schemahash. Klart: kall/varm laddning lika; stale cache nekas.

### Fas 2 — harness

Historiskt levererat: ARM-runner, diff/replay/run, oracletraces och screenshots.
Helspelsbevis återstår i **5A.02–06/5B.01**. Se [equivalence](docs/equivalence.md),
[ARM-runner](docs/arm-runner.md), [enginerunner](docs/engine-runner.md).

### Fas 3 — första bild

Historiskt levererat: tvåskärmsmodell, CPU-raster, wgpu-presentation och input.
Full intro/title/avancerade effekter: **8B.01–05** efter relevanta **5B**-delar.
Gamepad/livscykel: **9A/9B**. Befintlig OAM/text återanvänds.

### Fas 4 — data/save/new-game

Historiskt levererat: art/move/item-data, text/font, LCRNG/MT, savecontainer/init,
Oak/name och sovrumslandning. Funktionellt accepterad 2026-09-11. Se
[game flow](docs/game-flow.md), [save](docs/save.md), [text](docs/text.md).

- [ ] **4R.01** Jämför post-Oak hela relevanta block/regioner med lika extern input. Klart: båda avatarval, egen/default name och RTC-grenar; fältvis avvikelserapport.
- [ ] **4R.02** Retailimport från olika progressioner och bytebevarande export utan mutation. Klart: okända block bevaras; giltig men ostödd spelplats ger begripligt fel.
- [ ] **4R.03** Savefelmatris: en/båda slots korrupta, fel längd, counter wrap, extrachunks/varningsrouting. Klart: befintliga tester återanvända och container/scen jämförda.

## Fas 5 — verifierbar overworld, först en liten del

Första omfattning: **sovrum 64, hus 1F 63 och New Bark 60**. Elm/Route 29 kommer
med mon/battleberoenden. Renderad karta utan events är inte färdig spelbar karta.

### 5A — reproducerbar grund

- [ ] **5A.01** Frys baseline: revision/diffhash, ROM/pret/oracle/patch och befintliga kommandon. Klart: reproducerbart startläge utan förlorade lokala ändringar.
- [ ] **5A.02** Testläge med uttryckligt ROM/save/oraclekrav. Klart: saknat obligatoriskt underlag ger misslyckad körning; passed/failed/skipped och jämförelseart redovisas.
- [ ] **5A.03** Offentlig ROM-fri CI med låst build/format/syntetiska tester och separat betrodd ROM-verifiering. Klart: faktisk körning av båda, inga privata råartefakter i offentlig CI.
- [ ] **5A.04** Mät/inför tidsmodell för VBlank, speluppdatering, input, rendering, RTC och bootseed. Klart: boot/idle/turn/walk/fade jämförs utan scenvisa efterhandsförskjutningar.
- [ ] **5A.05** Versionssätt replaymanifest/observationsschema. Klart: revision/patch/initialsavehash/RTC/profil/frames/tidsmappning kontrolleras; inkompatibla jämförelser nekas.
- [ ] **5A.06** Engine/oracleobservationer för karta/position/riktning, flags/vars, objekt och script/waits, en familj per kort. Klart: injicerat positions-/flaggfel hittas vid första provpunkt även med lika RNG.
- [ ] **5A.07** Samla live-saveägande, dirty revision och persistenspolicy. Klart: stillastående ändring överlever omstart, checkpoint ändrar inte RNG och Save/autosave hålls reproducerbara.
- [ ] **5A.G** Delmilstolpe: baselinekörning och avsiktlig avvikelse visar att kedjan fungerar; obligatoriska tester faktiskt körda.

### 5B — gemensam grafikgrund

Många mekanismer finns i arbetskopian. Uppgifterna avser komplettering och
originalverifiering. Se [gfx](docs/gfx.md), [oracle](docs/oracle.md), research R2–R3.

- [ ] **5B.01** Generera raw-rörelsesekvenser från input/save med varje VBlank, även upprepad visning. Klart: sovrum/hus/New Bark i flera riktningar; mappning bestäms före diff.
- [ ] **5B.02** Verifiera fixed-point/FX-tabell/kamera/viewport. Klart: gränsvärden och reproducerbar tabell/frame över OS/arkitektur; värdens `f64::sin/cos` i build-scriptet utredd.
- [ ] **5B.03** Inventera/slutför GX-kommandon och position/normal/texture-matrix stacks. Klart per familj: reset/concat/stack/gränsfall och kommandospår jämförda.
- [ ] **5B.04** Verifiera sexplansklippning/culling/winding/primitiveidentitet/polygonordning. Klart: riktade kantfall och fältsekvenser utan nya täckningsfel.
- [ ] **5B.05** Verifiera material/normal/fyra ljus, ambient/diffuse/specular/emission/toon/shininess. Klart: per-command state och område/tidsljus mot rawoutput.
- [ ] **5B.06** Slutför native sampling per texturtyp, även DIRECT/COMP4x4, alpha/palett/repeat/flip/texture matrices. Klart per format: kanter/transparens/wrap jämförda.
- [ ] **5B.07** Verifiera Z/W/lika djup/polygon-id/translucens/depth-write. Klart: färg/alpha/djup/attribut utan dold normalisering.
- [ ] **5B.08** Verifiera skuggor/fog/edge marking/AA separat och ihop. Klart: rawytor/förändringsmasker matchar namngivna fall; retail `DISP3DCNT=0x0039` ingår där uppmätt.
- [ ] **5B.09** Återställ saknad submission/animation: markskugga, vindkvarnsblad, objekt-offsets och inventerade Nitro-animationer. Klart: orsak i asset/state/GX, inga positionsspecifika pixelpatchar.
- [ ] **5B.10** Mät simulation/laddning/raster/komposition/upload/presentation/backlog. Klart: reproducerbar releaseprofil, hårdvara, p50/p95/p99/max och kalla/varma kartbyten.
- [ ] **5B.G** Delmilstolpe: personlig granskning av konsekutiva bilder vid stolar/bänk/Moms bord och flera utomhuspositioner. Exakta gates noll diff; generell GX-täckning redovisas separat.

Historisk mätning 2026-09-12: tio sovrumsbilder hade 29–63 färg-, 0–5 djup- och
28–39 attributskillnader/bild, noll täckningsskillnader och en felpixel i sista
förändringsmasken. Sekvensen är utvalda bildpar, inte alla VBlank. Historiskt
New Bark raw-3D: 120 bilder, mean 4.113 ms, p95 5.191 ms, max 6.072 ms;
omfattar inte hela 16.714 ms-budgeten. Detta är öppna gap, inte nytt facit.

### 5C — första interaktionen och Mom

Befintligt: flaggfiltrerade objekt, sprite/model-cache, tick/animation och VM.
Se [arbetskorten](docs/planning/next-slices.md) för närmaste kontrakt.

- [ ] **5C.01** Live host med auktoritativ save/tempflags/RNG/task/waits. Klart: scriptmutation syns i NPC-synlighet/trace/save utan konkurrerande kopior.
- [ ] **5C.02** A-targeting: facing/höjd/räckvidd, objekt/BG-prioritet och counter-regler. Klart: träff/felvänd/miss/blockering, inga dubbelstarter vid hållen A.
- [ ] **5C.03** Öppna/skriva/vänta/stänga dialog genom TextPrinter. Klart: riktig ROM-dialog från A till återgiven kontroll med rätt waits/format/inputkanter.
- [ ] **5C.04** Yes/no och små valmenyer. Klart: cancel/resultatvariabel och bytecoderetur på rätt tick för pad/touch.
- [ ] **5C.05** Lock/facing/ApplyMovement/WaitMovement/release. Klart: player + två objekt, sista objektet styr väntan, inga läckta locks efter fel.
- [ ] **5C.06** Scriptade fades/warps/child tasks/return-to-field. Klart: caller fortsätter enligt retailtaskordning och destination init en gång.
- [ ] **5C.07** Autonom rörelse per nått NPC-typ-ID: look-around först, wander sedan. Klart: RNG/paus/gränser/dynamisk kollision jämförda.
- [ ] **5C.08** Samla scriptheaderparser, koppla ON_TRANSITION/ON_RESUME/ON_LOAD/frame table. Klart: ordning vid map entry/Continue/appretur, synlighet efter flaggändring.
- [ ] **5C.09** BG/skylt/koordinattriggers. Klart: riktning/höjd/villkor/prioritet, ingen omstart varje tick när spelaren står kvar.
- [ ] **5C.10** Moms ROM-scen och alla New Bark-skyltar med live host. Klart: Pokégear-mottagning/flags/std-scripts/waits; återinträde/Continue upprepar inte scenen.
- [ ] **5C.G** Delmilstolpe: sovrum → Mom → utgång → skylt → Save/omstart, state/RNG jämförda och manuell regression.

Ljudkommandon får först registrera händelser. Sound waits måste ha originalets
livslängd och namngivet beroende på 7A:s scheduler. ”Ljud klart direkt” kan inte
godkänna tidsparitet. Mätningen kan leda till ett tidigare 7A-arbetskort.

### 5D — UI, klocka och presentation

- [ ] **5D.01** StartMenu till field task/input/retur. Klart: X/B/touch, hållen öppningsknapp och faktisk START-semantik; ingen rörelse genom menyn.
- [ ] **5D.02** Undre LCD, låsta/synliga ikoner, bestående markör, OAM-dimning. Klart: före/efter Mom/full meny mot oracle; saknad app ger namngivet utvecklingsfel.
- [ ] **5D.03** Retail Save-dialog/cancel/skrivning/retur. Klart: generation/omstart med 5A.07; export/checkpoint ger inte dubbel retail-skrivning.
- [ ] **5D.04** Verifiera områdes-/dagsljus, props och musik-ID med gemensam RTC. Klart: före/vid/efter tidsgränser, midnatt/kartbyte; tint ersätter inte materialljus.
- [ ] **5D.05** Platsnamnspopup. Klart: map section/namnbank/återinträde/varaktighet/layering jämförda.
- [ ] **5D.G** Delmilstolpe: meny/Save/Continue/tidsljus i samma fältflöde. Bag/partyappar kräver 6A/6D.

### 5E — kartgränser och rörelse

- [ ] **5E.01** Flytta 3×3-fönster över cellgränser med bevarad objekt/eventidentitet. Klart: kanter/hörn fram/tillbaka utan hål, dubbla NPC:er eller cacheberoende RNG.
- [ ] **5E.02** Höjd-/objektkollision: `sub_02054954`, ockuperad/reserverad tile, mötande NPC, saknad höjd. Klart: riktade originalfall; brohöjder får kort före konsumenten.
- [ ] **5E.03** Warpvarianter var för sig: dörr in/ut, trappa, ladder, escalator, panel, dynamiskt `0x100`-ankare. Klart per variant: destination/facing/fade/waits/retur; mät dörr-ut i stället för antagande.
- [ ] **5E.04** Running shoes/B/touch-toggle och ledge/hopp separat. Klart: progression/steg/animation/envägskollision/event/encounter på rätt steg.
- [ ] **5E.05** Elm → Route 29 → Cherrygrove → Route 30 i kartkort. Klart per karta: scripts/NPC/events/bakåtväg/save/Continue/encounters/battle utan storybypass; kräver 6B/6C där nått.
- [ ] **5E.06** Bicycle mount/speed/turn/dismount/områdesspärrar. Klart: save/kartbyte/meny och rörelse i normal-/kantfall.
- [ ] **5E.07** Surf enter/move/exit/behörighet. Klart: strand/avatar/follower/vattenencounter/battle-retur; övriga fältmoves i kartberoende 8A-kort.
- [ ] **5E.G** Delmilstolpe för namngiven kartmängd: passage i båda riktningar och sparad återstart utan ostödda nåbara kommandon.
- [ ] **5.G** Hela fasen accepteras när alla 5A–5E-löv är stängda. Mindre delar accepteras tidigare med respektive G. Ingen ”hela Johto”-etikett här.

## Fas 6 — Pokémon, encounters och battle

Ordning: data → starter → encounter → första slag → avslutad battle → fångst/
menyer → bredd. Alla moves behövs inte för första slaget. Saknat stöd utanför
aktuell slice ger namngivet fel och finns i täckningsmatrisen.

### 6A — spelobjekt och save

- [ ] **6A.01** Inventera BoxMon/PartyMon och accessorprober. Klart: fields/widths/offsets/shuffle/kryptering/checksummor med ROM-pins.
- [ ] **6A.02** Läs/skriv mon utan mutation. Klart: alla 24 shuffleordningar, tom/egg/korrupt checksumma, originalaccessorer och byteidentisk roundtrip.
- [ ] **6A.03** PID/IV/EV/nature/ability/gender/form/shiny-accessorer per grupp. Klart: mutation ändrar rätt payload/checksumma; shinykriterium skiljs från genererings-RNG.
- [ ] **6A.04** Stats/level/experience. Klart: integeravrundning/HP-specialfall/gränser mot originalet.
- [ ] **6A.05** Party add/remove/reorder/full. Klart: count/HP/status/save konsekventa efter app-/battle-retur.
- [ ] **6A.06** Bagpockets/stackar/key items/TM/HM/capacity. Klart: give/remove/full/zero/gränser/scriptresultat mot originalet.
- [ ] **6A.07** Pokédex seen/caught/forms. Klart: encounter/gift/catch ger rätt bits/save före UI.
- [ ] **6A.08** PC deposit/withdraw/move. Klart: full box/party, sista tillåtna partymedlem, held item/mail bevaras.
- [ ] **6A.09** Koppla strukturerna till live-save/trace. Klart: retailimport/export och accessorjämförelse; PKHeX sekundär kontroll enligt R5.
- [ ] **6A.G** Delmilstolpe: mon och item skapas genom riktig spelfunktion, sparas/laddas och läses identiskt av originalaccessorer.

### 6B — starter, follower och encounter

- [ ] **6B.01** Elms scripts till starterval. Klart: banker/hosteffekter inventerade, inga placeholder-resultat från `launch`.
- [ ] **6B.02** Starterapp och mon-generering. Klart: varje val/cancel/confirm och PID/IV/TID/RNG-ordning mot originalet.
- [ ] **6B.03** Nickname/gift/Pokédex/party till scriptretur. Klart: egen/default name och Save/Continue utan dubbel gåva.
- [ ] **6B.04** Första follower: spawn/gångkö/sväng/warp/hide/show. Klart: första rutten utan kollision/RNG-gap; full bredd i 8C.
- [ ] **6B.05** Encountertabeller och karta-/tidsvarianter. Klart: ROM-rader/modifierare jämförda, inga handskrivna artlistor.
- [ ] **6B.06** Steg/encountercheck och Repel/lead-modifierare för första gräsfallet. Klart: trigger/no-trigger och RNG vid tidiga returer.
- [ ] **6B.07** Vild mon/BattleSetup. Klart: slot/level/PID/IV/nature/ability/item/form/gender/shiny/RNG, en modifierarfamilj per kort.
- [ ] **6B.08** Battleinträde/fältretur. Klart: rörelse/script/follower pausas/återställs; samma encounter startar inte om.
- [ ] **6B.G** Delmilstolpe: starter → lämna labbet → encounter med lika mon-/RNG-state.

### 6C — första avslutade strid, sedan mekanikbredd

- [ ] **6C.01** Battle-C/asm-inventering och command/subscript-format. Klart: controller/VM/data/presentation/prober kartlagda; saknade symboler har mätkort.
- [ ] **6C.02** BattleSetup/BattleState och deterministisk command/eventström. Klart: battler/party-index/snapshot/field-retur separerade från renderer/inputenhet.
- [ ] **6C.03** Move/target/trainerdata och en vanlig fysisk move. Klart: samma command från UI/headless; första UI tas från 6D.01.
- [ ] **6C.04** Actionorder: switch/item/move/priority/speed/ties. Klart: normalfall/lika speed/relevanta undantag med lika RNG.
- [ ] **6C.05** Accuracy/critical/damage i separata kort. Klart: avrundning/overflow/miss/STAB/type/variation/immunitet efter varje delsteg mot originalet.
- [ ] **6C.06** Första damage-moven genom battle scripts till HP/PP/text/faint. Klart: hela slagets eventordning, även miss.
- [ ] **6C.07** Enkel vild strid till ny turn/seger/flykt/förlust/blackout. Klart: XP/pengar/party/fältretur utan fastnad task; minimal XP från 6E.03 levereras här.
- [ ] **6C.G1** Första battle accepterad: encounter från field till field, Save/Continue och lika state/RNG.
- [ ] **6C.08** Stat stages och burn/poison/toxic/paralysis/sleep/freeze per kort. Klart: start/duration/turhinder/residual/cure/order.
- [ ] **6C.09** Volatila effekter per familj: confusion/flinch/trapping/protect/substitute/charge/recharge/multi-turn/multi-hit. Klart: avbrott/switch/faint-reset/RNG.
- [ ] **6C.10** Abilities per triggerfamilj: switch-in, accuracy/damage, kontakt/status/end-turn. Klart: order/suppression/redirection där mekaniken kräver det.
- [ ] **6C.11** Held items/battleitems per familj. Klart: trigger/konsumtion/misslyckande/save; ingen dubbel konsumtion vid presentationsavbrott.
- [ ] **6C.12** Weather/screens/hazards/fälteffekter per kort. Klart: duration/order/residual/faintkedjor/switch-in.
- [ ] **6C.13** Trainer-AI, första rivalen först. Klart: kandidatpoäng/val/switch/item/RNG mot retail controller.
- [ ] **6C.14** Doubles/multi/tag som separata battletyper. Klart: targeting/spread damage/partner-AI/samtidiga faints/ersättare.
- [ ] **6C.15** Täckningsregister för alla US-HG moves/effect scripts/abilities/items/battletyper. Klart: originalkontroll per nådd effekt, kombinationsrisker egna fall och luckor egna ID.
- [ ] **6C.G2** Battlebredd: deklarerad matris komplett, trainer/single/double-replay med state/RNG/eventparitet.

### 6D — presentation och spelappar

- [ ] **6D.01** Första battlebild/commandmeny från ROM tillsammans med första 6C-slicen. Klart: två LCD, övergång/battlers/text/val över en hel turn.
- [ ] **6D.02** HP/EXP-bars, PP/targetval, text waits, cries och intro/exit. Klart: rätt waits utan oavsiktliga gameplay-RNG-drag.
- [ ] **6D.03** Battle-animation commands per använd effektfamilj. Klart: sprite/3D/particles/palette/blend/camera/cleanup/waits i originalsekvenser.
- [ ] **6D.04** Partylist/summary, sedan reorder/switch. Klart: pad/touch/cancel/egg/fainted/full party/retur; data från 6A.
- [ ] **6D.05** Baglist/pockets, sedan use/give/toss/register, ett val per kort. Klart: field/battle/ogiltigt mål/fullstack/key item/cancel/riktig mutation.
- [ ] **6D.06** PC UI över 6A.08. Klart: boxbyte/flytt/item/mail/cancel utan oavsiktlig mutation.
- [ ] **6D.07** Pokédex/trainer card/options som egna appar. Klart: unlock/save/text speed/battle style/sound options/pad/touch.
- [ ] **6D.G** Delmilstolpe: script/field/battle använder samma data, riktiga returer och ingen inputläcka till bakomliggande scen.

### 6E — fångst och utveckling

- [ ] **6E.01** Ballval/catchformel/shakes per ballfamilj. Klart: miss/framgång/trainernekande/konsumtion/RNG.
- [ ] **6E.02** Catch till nickname/Pokédex/party/PC. Klart: full party/boxkapacitet/item/save och bibehållen mon-identitet.
- [ ] **6E.03** XP/EV/level-up/move learn per kort. Klart: deltagare/delning/flera levels/full moveset/cancel/presentation/saveordning; minimal XP med 6C.07.
- [ ] **6E.04** Evolution per triggerfamilj med UI. Klart: villkor/cancel/form/ability/stats/move learn; tradeintegration beroende på 8C.
- [ ] **6E.05** Friendship/egg steps/hatch/daycare som egna kort. Klart: step/RTC/RNG/full party/överlämning.
- [ ] **6E.06** Breeding generation/inheritance. Klart: compatibility/PID/IV/move/ability/pickup/save mot originalprober.
- [ ] **6.G** Fas accepterad när alla delkontrakt verifierats. Ett storyegg bockar inte av hela breeding.

## Fas 7 — ljud i små hörbara steg

SDAT-grunden löser containerstruktur. ROM-inventeringen anger SSEQ/SBNK/SWAR
och inga STRM/SEQARC-poster i de två arkiven. STRM blir uppgift först vid
identifierad konsument. Se [SDAT](docs/nitro-sdat.md), research R4/R7.

### 7A — första sekvensen

- [ ] **7A.01** Inventera commands/banker/sampleformat/cries/konsumenter och gör avgränsat teknikval. Klart: egen kärna/bibliotek bedömda för determinism/licens/plattform/luckor.
- [ ] **7A.02** AudioCommand/State, sequencerklocka och korrekt tickande nulldriver. Klart: sound waits/BGM/ME-status utan ljudenhet; inga oavsiktliga gameplay-RNG-drag.
- [ ] **7A.03** SWAR/SWAV PCM8/PCM16/ADPCM per kort samt loop/pitch/timer. Klart: syntetiska gränsfall och lokala originaljämförelser.
- [ ] **7A.04** SBNK-instrument/key-/velocity ranges och använda PSG/noisefall. Klart: instruments note-on/off/envelope mot referens.
- [ ] **7A.05** En SSEQ med note/rest/program/tempo/end; sedan loop/call/branch/variables/random/multitrack per kort. Klart: eventposition/waits, fel med offset för ostött kommando.
- [ ] **7A.G** Delmilstolpe: en fältlåt och ett SFX hörs; start/stop/wait verifieras headless.

### 7B — mixer och prioritering

- [ ] **7B.01** Channel allocation/priority/preemption/voice limits. Klart: samtidiga BGM/ME/cry/SFX och stulen kanal enligt referensen.
- [ ] **7B.02** Envelope/pan/volume/pitch bend/LFO/mixeraritmetik per kort. Klart: avrundning/saturering/jämförelsepunkt; extern spelare sekundär referens.
- [ ] **7B.03** Kartmusik/dag-natt/jinglar/cries/battle/BGM-retur. Klart: avbrott/nästlade jinglar/fades/restart/scene waits.
- [ ] **7B.04** Desktoputmatning/resampling. Klart: callbackstorlek/mute/enhetsbyte påverkar inte simulation, underrun/latency mätbara.
- [ ] **7B.G** Delmilstolpe: new-game → Mom → field → battle → field med korrekt ljudordning och hörbar output.

### 7C — ljudparitet

- [ ] **7C.01** Event/sequence/channelstate på fast tidsaxel. Klart: ingen oförklarad drift i namngivna fall, även med avstängt ljud.
- [ ] **7C.02** PCM före värdens resampling/volym. Klart: fast format/tidsankare/längd, explicit exakthet eller motiverad tolerans; sampleexakt kräver noll diff.
- [ ] **7C.03** Långt loop-/scenbytestest. Klart: ingen timingdrift/kanal-/resursläcka/förlorad avslutshändelse.
- [ ] **7.G** Fas accepterad: validerat gameplayljud; eventparitet och PCM-paritet redovisas separat.

## Fas 8 — innehåll, precision och full omfattning

### 8A — en geografisk etapp åt gången

Varje etapp delas före start i **ett kort per karta/scen/battle/ny mekanik**.
Inventera assets, events, scripts/std-anrop, opcodes/hosteffekter, NPC movement
types, warps, trainers/encounters, musik och persistenta flags.

Acceptans per etapp: passage i båda riktningar, obligatoriska grenar/nekanden,
Save/Continue före/efter storyflagga, inget ostött nåbart beteende och minst ett
oracleförankrat replay. Ny mekanik får beroendekort före storyscenen. Geografin
nedan är en checklista; exakt obligatorisk eventordning hämtas ur ROM:s scripts.

- [ ] **8A.01** New Bark/Elm → Cherrygrove/Mr. Pokémon → retur/rival/fångsttutorial. Återanvänd 5E/6B; egg delivery/running shoes/Pokégear/returflags.
- [ ] **8A.02** Violet/Sprout Tower/Falkner. Egna kort för healing/center/shop, trainer sight, badge och storyegg där det nås.
- [ ] **8A.03** Union Cave/Azalea/Slowpoke Well/Bugsy/Ilex. Dungeonwarps/Rocket/Cut/Farfetch'd/spärrar.
- [ ] **8A.04** Goldenrod/Whitney/rutter. Bicycle/daycare/department store/radio/blockerande events.
- [ ] **8A.05** Ecruteak/Burned Tower/Morty/ruttgrenar. Roamers/legendflags/encounterpersistens.
- [ ] **8A.06** Olivine/Lighthouse/Cianwood/Chuck/Jasmine. Surf/Strength/medicine/sea routes/återbesök.
- [ ] **8A.07** Mahogany/Lake of Rage/Pryce/Rocket Hideout/Radio Tower. Specialencounter/disguise/puzzles/forced battles/returordning.
- [ ] **8A.08** Ice Path/Blackthorn/Clair/Dragon's Den/HeartGolds legendkedja. Isrörelse/Waterfall/Whirlpool/draktest/kimono-/Ho-Oh-scener enligt scripts.
- [ ] **8A.09** Route 27/26/Victory Road/Elite Four/Lance/credits. Stridskedjor/restriktioner/Hall of Fame/save/restart.
- [ ] **8A.10** Kanto i ett stad/gymgren-kort åt gången: S.S. Aqua, Power Plant, radio/Snorlax, transport/alla badges; giltig icke-linjär ordning.
- [ ] **8A.11** Blue/Mt. Silver/Red/postgame-unlocks. Klart: progression och återstart; credits är inte hela kampanjmålet.
- [ ] **8A.G** Kampanj accepterad: obruten ny-save-genomspelning plus korta delreplays; importerade checkpoints ersätter inte obrutet bevis.

### 8B — kvarvarande precision

- [ ] **8B.01** Full intro, en saknad scen per kort: skip/circle wipe/animation/musik. Klart: oskippad och tidigt/sent skippad sekvens mot originalet.
- [ ] **8B.02** Full title: Ho-Oh/3D/kamera/prompt/idle/timeout/loop. Klart: båda LCD/inputretur över flera animationstakter.
- [ ] **8B.03** Oak/naming exakt: nested overlay/fade/waits/yes-no-cursor/glow/wiggle/BACK/OK. Klart: samma input/RTC/scen-/bildsekvens; 4R.01 för initdata.
- [ ] **8B.04** Inventera nådda affine/OAM/window/blend/mosaic/ext palettes/display capture/screen swap. Klart: originalsekvens per använd funktion i intro/battle/apps.
- [ ] **8B.05** GX-gates för battle/title/särskilda karttyper. Klart: bevis per använd GX/material/animationstyp; sovrum räcker inte för generell paritet.
- [ ] **8B.06** Savejämförelse vid explicita retail Save per etapp. Klart: lika initialsave/skrivsekvens/RTC; fullbyte när kontrollerat, annars semantisk rapport med öppna bytegap.
- [ ] **8B.07** Långspels-RNG/state med tät sampling kring första avvikelse. Klart: ingen hard region divergerar; sluthash döljer inte tidigare tillfälligt fel.
- [ ] **8B.08** Stäng driftundantag individuellt. Klart: orsak/region/intervall/ID för återstående; gameplay/RNG/save flyttas inte till drift för att passera.
- [ ] **8B.09** Långa sessioner/kartbyten/app-/battlereturer. Klart: angivna minnes-/latencygränser utan state/resource/taskläckor.
- [ ] **8B.G** Precision accepterad: maskinläsbar täckning/avvikelselista och paritetsanspråk som motsvarar bevisen.

### 8C — originalomfattning som inte får glömmas

Varje rad börjar med inventering av modes/banker/hardware-/saveberoenden och
testinput. Dela sedan i kort per funktion. Tidiga storyslices återanvänds.

- [ ] **8C.01** Full follower: form/spriteklasser/terräng/warp/interaktion/friendship/fynd/gåvor/events.
- [ ] **8C.02** Pokégear map/phone/radio: kontakter/scheman/samtal/kanalvillkor/musik/encounters.
- [ ] **8C.03** Kalender: veckodagar/resets/apricorns/swarm/lottery/rematches/clock-change/penalty/offlineförlopp, en feature per kort.
- [ ] **8C.04** Återstående fältmoves/encounters: fishing/Rock Smash/Headbutt/Safari/roaming/lead/form/time-modifierare.
- [ ] **8C.05** Bug Catching Contest/Safari Zone/Pokéathlon var för sig: entry/gameplay/resultat/belöning/save/RTC.
- [ ] **8C.06** Battle Frontier/facilities: regler/lag/streaks/AI/battletyper/rewards/extrachunks.
- [ ] **8C.07** Voltorb Flip/Game Corner/priser/shops/tutors/fossils/mail och övriga samlar-/utbytesfunktioner; US-specifik inventering.
- [ ] **8C.08** In-game trades/gifts/statiska legendarer/forms/postgame: villkor/RNG/respawn/Pokédex/save.
- [ ] **8C.09** Local wireless/trade/battle/Union Room: transport/två spelares state/avbrott/RNG; separat acceptans från singleplayer.
- [ ] **8C.10** Pokéwalker/IR, mikrofon och GBA-slot/Pal Park där ROM använder dem: externa input och lokal testmetod före implementation.
- [ ] **8C.11** Mystery Gift/nätfunktioner: klientformat/lokala protokollfall; externa tjänstberoenden får separat scopebeslut. Inga eventdata i repo.
- [ ] **8C.12** Fullständighetsinventering maps/scripts/apps/assets/moves/items/species/forms/externa funktioner. Klart: verifierings-ID eller öppet scopegap per post, inga tysta placeholders.
- [ ] **8.G** Full originalprofil accepterad när 8A–8C och deklarerade hårdvaruberoenden är stängda. Annars ange uppnådd delprofil, exempelvis kampanjparitet.

## Fas 9 — plattformar och leverans

Bas-CI kommer i 5A. Plattformsglue innehåller ingen game logic. Tidiga build-/
livscykelprover får upptäcka hinder; release kräver den deklarerade profilens gates.

### 9A — desktop

- [ ] **9A.01** Låsta Windows/Linux/macOS-builds och dependencies. Klart: faktisk CI och x86_64/ARM64-bevis där stödda.
- [ ] **9A.02** ROM-picker: rätt/fel dump/saknad cache/parsefel. Klart: begripligt fel/retur, inga assets i paket.
- [ ] **9A.03** Save import/export/backup/katalogpolicy. Klart: skrivfel/recover/read-only/fel fil utan skadat original; regressioncheckpunkt tydlig.
- [ ] **9A.04** Keyboard/gamepad/rebinding/focus-loss/mus→stylus. Klart: input släpps vid fokusförlust, LCD/skalning/samtidiga knappar/replay korrekt.
- [ ] **9A.05** Resize/DPI/fullscreen/device-loss/ljudenhetsbyte. Klart: samma coretrace vid olika presentationstakt och återhämtning utan state-reset.
- [ ] **9A.G** Acceptans per OS: installera/ROM/new-game/Continue/battle/save/exit/omstart från paket.

### 9B — Android

- [ ] **9B.01** Tunt Androidskal och låst NDK/build/ABI. Klart: appstart/samma headless-corefall, inga desktopantaganden i core.
- [ ] **9B.02** Systemets filåtkomst för ROM/save och kvarvarande behörigheter. Klart: återöppning efter omstart, återkallad åtkomst återställbar.
- [ ] **9B.03** Pause/resume/surface/rotation/process death. Klart: definierad/versionerad checkpoint; retail `.sav` utlovar inte godtycklig midbattle-suspend.
- [ ] **9B.04** Två LCD-layouter/stylus/skärmknappar. Klart: samtidig multi-touch, OS-avbrott släpper input; gamepad separat.
- [ ] **9B.05** Ljudfokus/avbrott/enhetsbyte/prestanda/termik. Klart: referensenheter/lång session utan gameplaydrift vid presentationstapp.
- [ ] **9B.G** Faktisk enhet: installera/ROM/Continue/suspend/process death/återstart/export/import.

### 9C — release

- [ ] **9C.01** Paket/checksummor/versioner/instruktioner/dependency notices. Klart: inga ROM/save/raw-dumpar/referensrepon/debugpaths i artefakten.
- [ ] **9C.02** Reproducerbar felrapport med build/profil/input. Crashrapportering opt-in och utan ROM/save/raw memory.
- [ ] **9C.03** Releasematris mot låst corpus. Klart: platform × profil × testläge, faktisk täckning/known issues och save/omstartprov.
- [ ] **9.G** Plattformsmålet: installbara artefakter för alla fyra OS med verifierad deklarerad funktionstäckning.

## Fas 10 — efter specificerad 1:1

- [ ] **10.01** Extensionsprofil avstängd i paritymode.
- [ ] **10.02** En QoL/layoutförändring åt gången; paritykorpusen fortsätter passera.
- [ ] **10.03** Högre upplösning/framerate med separerad presentation/simulation; ändrad gameplay kräver separat profil.
- [ ] **10.04** Versionerat mod/script/datagränssnitt och nytt innehåll när kärnkontraktet är stabilt.

## Valideringskontrakt och corpus

Fall behöver input, initialsave/RTC/maskinprofil, revisionsmanifest,
förväntade region-/eventhashar och exakt vad det bevisar. Rawbilder/state/PCM
stannar lokalt. Nya manifest-/tracefält är planerade i 5A, inte befintlig CLI.

| Nivå | Bevis | Underlag |
| --- | --- | --- |
| U | Rustlogik/format | Syntetiska data, inget ROM. |
| R | ROM-data/accessorer/ARM-prober | Rätt dump/pins; faktisk körning. |
| O | Oracle mot oraclebaseline | Samma version/profil/patch/input. |
| E | apricorn mot originalet | Gemensam tidsaxel, lika initialstate/schema. |
| V | Visuell/raw/PCM | Deklarerad yta/sampling; visuella fall även manuellt. |
| P | Paketerad plattform | Faktisk plattform/input/persistens/livscykel. |

Första corpusutökningen, ett litet fall per leverans:

- Behåll `boot-idle`/`new-game`; korrekt enginejämförelse efter 5A.04.
- Sovrum: idle/turn/walk/blocked step/warp med state/konsekutiva rawbilder.
- Hus 1F: Mom/återbesök/Continue. New Bark: varje skylt/NPC/meny/Save.
- Elm/Route 29: starter/follower/no-encounter/encounter/första battle.
- Battle: miss/hit/critical/faint/win/run/loss, sedan en mekanikfamilj i taget.
- Varje senare karta/app/ljudsystem: lokalt fall plus koppling till längre replay.

Hard regions: RNG, relevant state/save och kontrakterad task/eventordning.
Driftundantag anger orsak/region/intervall/ID/gräns. Kanonisk serialization,
aldrig Rustpadding/pekaradresser/slumpmässig map-ordning. Helsave-byteidentitet
kräver lika initial kortbild, skrivsekvens och RTC; semantiska block är delbevis.

Befintliga körvägar att verifiera/dokumentera vid 5A.01:

```powershell
cargo run -p apricorn-desktop -- --rom hg_usa.nds
cargo run -p apricorn-desktop -- --rom hg_usa.nds --save out/regression.sav
scripts/replay.ps1 corpus/boot-idle
scripts/shots.ps1 corpus/new-game "4816"
cargo run -p apricorn-harness --bin apricorn-gx-diff -- --sequence oracle/sequences/bedroom-down.tsv
```

Rawdiff kräver lokala filer enligt [oracleinstruktionerna](docs/oracle.md).
Avvikelse är ett fynd, inte skäl att skriva över baseline. På Windows kan GNU-
toolchain behöva `C:/msys64/mingw64/bin` på PATH. `oracle/setup.ps1`,
`oracle/setup.sh` och `scripts/replay.sh` finns för respektive bygg-/körväg.

**Manuell regression behåller persistent save.** Standard `apricorn.sav`;
separat `--save`-kopia vid experiment, `--no-save` bara för avsiktligt färskt
flöde. ROM/användarens originalsave får inte vara fixture-skrivmål. Återskapa
inte karaktären rutinmässigt. Gamla instruktioner om ännu ej mergad fas 5-gren
är ersatta; aktuellt startläge fastställs i 5A.01.

## Risker och beslut som styr nästa lilla uppgift

| Risk | Motåtgärd |
| --- | --- |
| Fel tidsaxel gör timing/RNG-felsökning missvisande. | 5A.04 före nya tidsparitetsanspråk. |
| Host/opcode misstas för spelbar scen. | 5C och karta→opcode→hosteffekt→replayregister. |
| Duplicerad save/RTC/RNG divergerar vid retur. | 5A.04/07, 5C.01: auktoritativ state. |
| Positionsspecifika grafikpatchar. | 5B: gemensam GX/assetdiagnos, flera scener. |
| ARM-runner saknar hårdvarubeteende. | Inventera probberoenden; waits/IPC i melonDS, inga påhittade no-ops. |
| Nya kartor växer omfånget osynligt. | Inventering före kartkort; nytt format/opcode/mekanik får ID. |
| Grönt test döljer saknad ROM/fel oracle. | 5A.02/05 och exekveringsbevis. |
| Save-bytekrav krockar med autosave/RTC. | 5A.07/8B.06 skiljer snapshot från lika retail-skrivsekvens. |
| Mobil avslöjar sena CPU/minnes/floating point-problem. | 5B.02/10 och tidigt 9B-experiment. |
| Nät/tillbehör saknar körbar motpart. | 8C.09–11: konkret beroende/profil; full 1:1 förblir öppet. |

Faktisk struktur: `apricorn-core` = headless state/assets; `apricorn-gfx` = CPU-
raster; `apricorn-harness` = prober/oracle/replay; `apricorn-tools` = data-CLI;
`apricorn-desktop` = input/pacing/persistens/presentation. `apricorn-audio` och
`apricorn-android` är planerade och ännu inte workspace-crates.
