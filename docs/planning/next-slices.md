# Arbetskort för nästa utvecklingsdelar

Hör till [PLAN](../../PLAN.md). Alla kort nedan är **planerade**; inga
implementationer godkänns av denna dokumentationsändring. Kod som redan finns
ska verifieras/återanvändas. Ett kort är en leverans, en delmilstolpe flera kort.

## Regler för detaljeringsnivå och spårbarhet

PLAN äger huvuduppgiftens kryss. Om ett kort delas får underuppgifter bokstavs-ID
här. Huvudkrysset stängs först när barnen är verifierade. Dubbelbokför inte samma
utförda ändring. En ny kunskapslucka resulterar i en mätuppgift, inte en gissad
retailkonstant. Första kön är **5A.01 → 5A.02a → 5A.02b**, därefter 5A.03/04.

Resultatrad vid avslut:

```text
ID / integrerad revision / ROM-profil / referensrevision + oraclepatch
Kontrakt / ändrade moduler / exakta verifieringskommandon
U,R,O,E,V,P: körda och godkända / misslyckade / överhoppade
Corpus och lokala råartefakter / första avvikelse eller zero diff
Manuell kontroll när relevant / kvarvarande öppna ID
```

Proposed symbol names och manifestfält är designförslag, inte befintliga API:er.
Binära offset-/tickvärden som saknas nedan måste mätas mot pinnad ROM innan kod.

## 5A.01 — reproducerbar baseline, ingen gameplayändring

**Resultat:** nästa utvecklare kan återskapa nuläget och skilja gamla fel från nya.
**Beroenden:** inga nya. **Ytor:** docs, scripts, corpus, befintlig arbetskopia.

Registrera HEAD, branch, diff-/untracked-filhashar, toolchain/Cargo.lock,
ROM-hash, lokal pret/melonDS-revision, oraclepatchhash samt tillgängliga privata
fixtures. Registrera filnamn/hashes, inte saveinnehåll eller privata sökvägar i
publika artefakter. Ändra inte användarens redan modifierade kod eller saves.

**Leverans:** en daterad lokal körningsrapport och portabelt recept i docs.
Återanvänd `oracle/setup.*`, `scripts/replay.*`, `apricorn-run` och rawdiff.
Markera vilka rawfiler som måste genereras och var. En full oraclekompilering
är bara nödvändig om befintligt bygge inte kan identifieras/reproduceras.

**Acceptans:** relevant befintlig baseline körd utan goldenuppdatering; kända
failures/skips listade; instruktionen skiljer oracle-vs-oracle, ARM-prob och
engine-vs-original. En ny checkout saknar normalt ROM/save/refs och säger det.
Mätningen i researchdokumentet ersätter inte denna exekvering.

## 5A.02 — tester som inte kan ge falsk paritetsstatus

**Ytor:** ROM-laddare i tester, harness och körscripts. Återanvänd befintliga
ROM-gates; skapa en liten gemensam hjälpare där det minskar duplicering.

- [ ] **5A.02a** Inventera varje testsuites krav och nuvarande skipbeteende. Leverera tabell suite → syntetisk/ROM/save/refs/oracle/ignored/performance.
- [ ] **5A.02b** Inför ett explicit strikt verifieringsläge. Saknat obligatoriskt underlag eller fel ROM-version ger felstatus; vanligt ROM-fritt läge får fortfarande hoppa över frivilliga suites.
- [ ] **5A.02c** Sammanfatta executed/passed/failed/skipped och U/R/O/E/V/P i körningsrapport. Ingen rubrik ”paritet godkänd” om erforderliga E-fall saknas.

**Testmatris:** ROM saknas; fel hash; rätt ROM men oracle saknas; savefixture
saknas; komplett underlag; ett avsiktligt jämförelsefel. Använd explicit fixture-
sökväg eller isolerat testläge, inte att flytta/radera användarens privata filer.
Gör inte ROM-fri testning beroende av privat data. Rapportering ska inkludera
ignored benchmarks separat; de förvandlas inte automatiskt till vanliga tester.

## 5A.03 — första CI, utan privata data

**Beroenden:** 5A.02:s klassificering. **Ytor:** nya workflows och docs.
**Leverans:** låst build/test för en första hostplattform och ROM-fria tester;
övriga desktopbuilds följer 9A.01. Spara status på formatkontroll och relevanta
syntetiska tester. Nya formatregler får inte skriva om orelaterad arbetskopia.

Den ROM-krävande körningen sker separat på betrodd lokal host med strikt läge.
Publicera inte ROM/save/PCM/rawbilder. Kör inte okänd PR-kod med tillgång till
privata fixtures. Knyt båda rapporterna till samma commit/arbetskopia.
**Acceptans:** först ett riktigt grönt syntetiskt jobb, sedan en verklig lokal
ROM-rapport. Saknas fjärr-CI-tillgång markeras den delen öppen, inte simulerat grön.

## 5A.04 — en mätt tidsmodell

**Beroende:** 5A.01. **Ytor:** `core/app/game.rs`, `field/system.rs`, `rtc.rs`,
desktop runner, harness engine/trace/input. **Referens:** `NitroMain`,
`FieldSystem_Main`, SysTask/fade-ordning; [field system](../field-system.md).

- [ ] **5A.04a** Mät power-on/seed/intro och field idle/turn/walk/fade: VBlank, inputlatch, logic update, render submission, presenterad bild och state sampling.
- [ ] **5A.04b** Skriv tidskontrakt med explicit global VBlank-räknare, speluppdateringsräknare och observerad schedulerordning per scenklass. Ange vilka upprepade bilder som är retailbeteende.
- [ ] **5A.04c** Implementera clock-/schedulergrund och korrekt input edge/repeat. Ingen global ”varannan tick” utan mätstöd för alla berörda scener.
- [ ] **5A.04d** Koppla bootseed/RTC/field lighting till samma klockkälla och uppdatera traces. Eliminera oberoende `ticks / 60`-klocka som källa till gameplaytid.
- [ ] **5A.04e** Kör boot/turn/walk/fade-replay och oförändrat input under olika hostpacing. Klart: samma corestate och rätt retailtidsaxel, inga efterhandsjusterade inputframes.

**Kontrakt:** håll värdens wall clock/presentation skild från simulation. RTC i
replay härleds från fast epoch och exakt definierad rationell tidsbas. Underlaget
nämner 560190 DS-cykler per bild; fastställ enhet/frekvens och avrundningsordning
före implementation. Decimalen 59.8268 i headern räcker inte för långtidstiming.

**Fall:** input press/release mellan logikticks, hållen input över fade, scenbyte,
korta/långa hoststopp, räknargräns, dygnsbyte. Kontrollera MT-state och cursor
separat. Bootgapet vid VBlank 185 är ett mätankare, inte en universell offsetfix.
Den riktade renderersekvensens ommappade bildpar får inte användas som tidsbevis.

## 5A.05 — traceidentitet och jämförelsekontrakt

**Ytor:** harness `trace/regions/replay/oracle/engine`, corpusmanifest.
**Beroenden:** 0R.02 och tidskontraktet 5A.04b; implementation kan börja före
alla gameplaykonsumenter av den nya klockan är färdiga.

Utöka med ett versionssatt manifest eller traceheader enligt minsta ändring:
ROM-ID, engine-revision + dirty-digest, oracle-revision/patch/build options,
inputhash, initialsavehash/blank-card-markör, RTC/firmwareprofil,
observationsschemahash, sample schedule och tidsdomän. Självbeskrivande
jämförelseart: oracle regression, function probe, engine parity, render diagnostic.

**Acceptans:** rätt par kan jämföras; ändrad save/input/RTC/schema/profil nekas
innan diff. Producenter får naturligtvis ha olika revisions-ID; de identifieras
och valideras mot manifestet, inte krävs vara samma binär. Gamla corpusfiler
migreras explicit, deras facit ändras inte automatiskt.

**Drift:** region + tidsintervall + orsak + uppgifts-ID + gräns; inga godtyckliga
globalt tolererade avvikelser. Missing/duplicate/out-of-order/truncated records
ska ge fel, inte ett kortare ”lika prefix”. Save/hash metadata har inga råbytes.

## 5A.06 — statejämförelse som faktiskt hittar gameplayfel

**Beroenden:** 5A.05; börja med fält som finns redan. **Ytor:** harness engine,
oracle watches och pinnade accessorer. **Input:** lika ROM/save/RTC/input.

- [ ] **5A.06a** Karta/warp/position/facing/player movement: specificera widths, signedness, fx32 och samplepunkt; lägg till båda producenterna.
- [ ] **5A.06b** Flags/vars/saveblock: specificera persistenta kontra temporära fält; berörda block serialiseras i kanonisk ordning.
- [ ] **5A.06c** Objekt och script: id/map/position/facing/movement, context/bank/PC/wait/lock/taskordning. Levereras när 5C ger konsumenten; behåll öppet tills dess.

**Protokoll:** heapbaserad retailstate nås via verifierad root/pointerkedja eller
instrumenterad accessor; använd inte antagna permanenta adresser. Hasha inte
pekare, oinitialiserad padding eller Rustminneslayout. Representera frånvarande
region med explicit fas-/availability-kontrakt, inte samma hash som nollstate.

**Acceptans:** injicera ett position-, flagg- och senare waitfel i testkopior.
Första felprov identifieras med region/fält och senaste lika provpunkt. RNG kan
vara oförändrad och jämförelsen ska ändå misslyckas. Tät sampling sker kring
triggers; glest samplade slutpunkter får inte påstå likhet mellan proven.

## 5A.07 — en live-save och pålitlig persistens

**Beroende:** baseline. **Ytor:** Game, FieldSystem, save, desktop persistens.
**Nuläge:** `card_snapshot()` klonar bas, kopierar flags/position och anropar
`save_game`; desktopens periodiska dirtykontroll tittar på platsändring.

- [ ] **5A.07a** Kartlägg ägarskap/livslängd för kortbild, aktiva block, new-game-state och fältkopior. Bestäm en auktoritativ live-state med tydliga lånegränser.
- [ ] **5A.07b** Inför dirty revision för persistenta mutationer även utan rörelse. Koppla flags först; senare bag/party använder samma väg.
- [ ] **5A.07c** Separera sidoeffektfri diagnostisk snapshot från explicita retail Save och regressionens persistenta checkpoint. Prova generation/slotrotation vid upprepade saves; mutera inte gameplay-RNG vid export.
- [ ] **5A.07d** Verifiera hostskrivningens temp/backup/recover-förlopp med isolerade filer. Behåll nuvarande bekväma persistenta regressionflöde.

**Acceptansfall:** stå still och ändra flagga → checkpoint/omstart; Continue →
ändra/spara två gånger; Cancel Save; snapshot två gånger utan spelmutation;
skrivfel före/efter rename; återhämtning med saknad primary/backup; fel format;
bevarade okända saveblock. Befintligt original får inte förstöras vid fel.
Exakt slotsemantik fastställs mot `save_game`/retail, inte Windowsfilens mtime.
Paritetsreplay använder lika initial kortbild och retail-skrivsekvens; autosave
som regressionhjälp finns kvar men får inte förorena dess bytejämförelse.

## 5B.01 — första tillförlitliga råsekvensen

**Beroenden:** 5A.01/04b/05. **Ytor:** `oracle/sequences`, GX-diff, oracle capture.
**Leverans:** ett genereringsrecept som återskapar sovrumssekvensen från startstate,
sedan separata kort för hus 1F och utomhus. Båda avatarvalen inkluderas över tid.

Jämför färg/alpha/djup/attribut/coverage och förändringsmask mellan varje par.
Registrera display flags, kamera, objekt/animation och frame samplepunkt.
Nuvarande `bedroom-down.tsv` behålls som diagnostiskt historiskt fall; det väljer
och upprepar oracleframes och bevisar därför inte kontinuerlig tidsparitet.

**Acceptans:** alla producerade bilder representerade på deklarerad tidsaxel;
ingen bortsortering av svåra bilder eller lokal offset som valts efter diff.
En flyttad bild ska ge fel även om stillbildshashar återkommer. Noll diffs krävs
för exakt gate; avvikelser får rapport med första pixel/kanal och kategori.
Markskugga vs rasterfel isoleras före ändring av algoritm. Personen som ändrar
renderern granskar själv konsekutiva inne-/utebilder före användaracceptans.

## 5C.01 — live host utan duplicerat gameplay-state

**Beroenden:** 5A.07, 5A.04:s schedulerkontrakt, 5A.06a/b. 5A.06c slutförs med
scriptkonsumenten och blockerar inte första hostkortet. **Ytor:** Game,
FieldSystem, script/host/env, save/vars_flags. Referens: `script_manager.c`,
`task.c` och [script VM](../script-vm.md).

En scriptkörning får ett kortlivat muterbart hostgränssnitt till auktoritativ
save/tempflags/RNG/objekt/UI/taskstate. Undvik klonade saveblock som måste
synkroniseras efter varje command och håll inga asset-storelås över nästa tick.
FieldAction/Query/WaitFor kopplas en familj i taget med riktig konsument.
Objektidentitet måste vara stabil över samma task; lookup får inte bygga på
Vec-index som ändras när ett annat objekt försvinner.

**Första leverans:** flagga/variabel/query + en stoppande wait i riktig field.
Inga nya generella callback-/pluginabstraktioner utan konsument.
**Acceptans:** två VM-contexts ser samma state, CallStd-slotordning bevaras,
RNG delas och mutation syns i snapshot/save. Ostött command ger bank/PC/opcode/
map/taskdiagnostik och tydligt fel, aldrig `success` eller obegränsad busy-loop.
Budgetvakt får ge diagnostiskt fel, inte ändra retailens yieldordning tyst.

## 5C.02 — hitta rätt samtalsmål

**Resultat:** ett A-tryck väljer precis det objekt eller event originalet skulle
välja. **Input:** nytryck, spelarposition/riktning/höjd, terrain/events/objekt,
aktiva tasks/locks. **Output:** högst ett interaction target med objekt/event-ID,
script/message bank och facing; ingen dialogrendering i detta kort.

Mät ordning i `FieldInput_Process` och dess targetfunktioner. Fastställ regler
för tile framför, disk/counter, höjd, osynlig/flaggborttagen NPC, busy field och
NPC mitt i steg. Gissa inte en generell euklidisk radie.

**Acceptansfall:** fyra riktningar; ryggen mot NPC; ingen träff; NPC bakom
hinder/counter; höjdskillnad; två konkurrerande events; held A; A under warp;
NPC moving/locked. Jämför valt ID, facingändring och RNG före/efter. Objektets
script startas först av integrationen i 5C.03; negativt fall lämnar state intakt.

## 5C.03 — en fullständig dialog från A till fält

**Beroenden:** live host/targeting/TextPrinter; **Ytor:** field, app/text,
script host, frame. Återanvänd font/message format/fade; bygg inte ny textrenderer.

Välj en enkel nåbar ROM-dialog vars command- och ljudberoenden är inventerade.
States: target/lock → window open → printing → await advance → close → release.
Detta är ett konceptuellt flöde; antal ticks/yields bestäms av originalet.
Formatteringen använder riktiga spelvariabler och bibehåller kontrollkoder.

**Acceptansfall:** text speed, rad/page-break, nedåtpil, lång sträng, spelarnamn,
A/B under utskrift, samma knapp hållen från öppning, stängning/nyöppning.
Fönstret ligger på rätt LCD/lager med rätt palette/blend; clear återställer
bakomliggande bild. Spelaren rör sig inte under lock och får kontroll igen.
Meddelandet har samma ID och expanderade kodpunkter, aldrig kopierad hårdkodad text.
Nytt script kan inte börja innan föregående caller är klar.

## 5C.04 — valresultat och cancel

**Beroende:** dialog. **Leverans:** ett yes/no-fall, därefter ett flerordsval.
Mät resultatsentinel, defaultval, cancelkod, touch hitbox och när native wait
släpps. Ett child-appresultat är inte samma sak som att öppningsanimationen är klar.

**Acceptans:** yes/no/B, upp/ner runt kanten, touch down/drag/release, hållen A
från föregående sida, cancel när otillåtet. Resultatvariabel ändras en gång,
meny resursstädas, bytecode återupptas rätt tick. Alternativa grenar måste köras;
en recordinghost som alltid väljer yes duger inte som livebevis.

## 5C.05–06 — rörelse och child tasks i små kort

**Referens:** ApplyMovement/WaitMovement, TaskManager_Call och native waits i
[script VM](../script-vm.md). Befintlig rörelsemaskin återanvänds.

- [ ] **5C.05a** Lock/facing/release för ett stillastående objekt; verifiera tidigare pause-state och återställning.
- [ ] **5C.05b** ApplyMovement/WaitMovement för ett objekt; flera repeatsteg, END och sista tickens animation mot originalet.
- [ ] **5C.05c** Player + två samtidiga objekt och wait på sista objektet; saknat/borttaget ID behandlas enligt retailens dokumenterade specialfall, övrigt ger diagnos.
- [ ] **5C.06a** Child fade: suspend/resume och samma-tick-retur enligt uppmätt TaskManager-ordning.
- [ ] **5C.06b** Scriptad warp: spara destination, ladda, init, återkomst; source-sceneobjekt får inte leva kvar felaktigt i caller.

**Gemensam acceptans:** rörelse/animation/RNG fortsätter eller pausas exakt som
referensen. Warp till saknad asset ger rapporterbart fel och konsekvent scen,
inte lyckad scriptretur med halv state. Inga dubbla fades, rörelse efter release
från ett annat contexts lock eller lås som blir permanenta efter fel. Låsägande
mäts för samtidiga tasks; inför inte godtycklig global bool-semantik.

## 5C.07 — autonom rörelse, ett typ-ID åt gången

Inventera New Bark/husets ObjectEvent movement types och de originalfunktioner
som de använder. Första kortet är en look-around-typ; nästa en wander-typ.
**Statekontrakt:** init timer, riktning, ranges/home position, RNG-drag, pause,
script takeover, blocked step, resume och last animation command.

**Prober:** kort sekvens som både väljer rörelse och blockeras, tillräckligt lång
idle för timerwrap/nytt val, NPC mot player/NPC, synlighetsbyte och återinträde.
Alla RNG-drag loggas med konsument/eventordinal i lokal diagnostik; rendering/
cache får inte dra gameplay-RNG. Samma map seed ska ge samma sekvens efter
replay, inte efter varje godtycklig cacheomladdning. Mer avancerad höjd-/bro-
kollision kompletteras av 5E.02 före första kartkonsumenten.

## 5C.08–09 — kartlivscykel och triggerordning

Samla `field/script_header.rs` och `script/header.rs` bakom en parser med
bevarade regressioner innan callbackkonsumenterna ändras. Inventory räknar
faktiskt reachable bytecode, även CallStd och dynamiska scriptval, inte bara
handlerantal. Det dokumenterade 155/853-talet är en tidigare snapshot.

- [ ] **5C.08a** Samlad parser med samma utdata för befintliga ROM-headerfall.
- [ ] **5C.08b** ON_TRANSITION/ON_RESUME/ON_LOAD: mät ordning relativt objektspawn, fade, Continue och meny/battle-retur; koppla en callbacktyp per ändring.
- [ ] **5C.08c** Frame table: villkor/variabler, första match, busy state och engångsscen. Flags syns i objektmanager samma kontrakterade punkt.
- [ ] **5C.09a** En BG-skylt, sedan alla New Bark-skyltars riktning/typ/prioritet.
- [ ] **5C.09b** Koordinattrigger: villkor/höjd/facing/step timing; samtidigt A/menu/warp och kvarstående spelare.

**Acceptans:** ny karta, återkomst, Continue och appretur ger rätt inituppsättning.
En scen körs inte igen när dess flagga är satt; en trigger som tillåts återkomma
är åter aktiverbar på retailens villkor. Jämför också konsekvenserna för NPC
synlighet och RNG, inte bara antal callbackanrop.

## 5C.10 — Moms scen som första spelbara delmilstolpe

**Beroenden:** nådda kort 5C.01–09, scriptstate i 5A.06c och ljudscheduler från
7A om sound waits kräver den. Beroendena kräver inte hela framtida subsystem.
**Start:** reproducerbart new-game i sovrummet eller samma pinnade pre-Mom-save.
**Slut:** scenen genomförd, kontroll åter, utgång nåbar, flags/save konsekventa.

Inventera bank 845/frame table och alla std-anrop för vald ROM. Specificera
riktiga var/flag/message-ID i kortets mätbilaga före implementation. Identifiera
Pokégearöverlämning och dialog-/rörelse-/ljud-/appkrav. En saknad app eller
hosteffekt skapar eget litet kort; injicera inte ”Mom already done” i mainflödet.

**Replays:** första besök, alla nådda valgrenar, återbesök, lämna/återvänd,
Save/Continue efteråt och båda avatarerna. Assertions på lås, NPC/playerposition,
flags/items/Pokégear-state, scenevar, script-PC/waits och RNG. Slutför scenens
statebevis med samma observationer i båda producenterna. Inspect båda LCD under
konsekutiva bilder, därefter användarens regressiontest. Först då 5C.G.

## 5D.01 — första startmenyn i riktiga fältet

**Beroenden:** host/task/inputkontrakt; befintliga `app/start_menu.rs` och dess
core/gfx-tester. Flytta inte bag/partyappimplementation in i detta kort.

Öppna med retailinput, låt menyn äga berörd input/task, stäng med uppmätt knapp
eller touch, returnera med rätt selection memory. `docs/menus.md` anger X/B
stänger och START inte gör det; extra plattformsgenväg måste skiljas från retail.

**Acceptans:** före/efter Mom, locked/unlocked entries, hold-open-knapp, cancel,
touch open/pick, menu medan script/warp busy, flera öppningar och save/appretur.
Samma knapppress får inte både stänga menyn och starta samtal eller rörelse.
OAM-dimning/undre LCD i 5D.02, riktig Save i 5D.03 och fulla appar i 6D får egna
verifieringskort. Ingen temporär tom app ska räknas som implementerad menyfunktion.

## Mall för nästa kort efter dessa

```text
ID och en konkret observerbar effekt:
Status: planerad / mätning pågår / implementerad / verifierad
Beroenden: exakta ID och vad som redan måste fungera
Originalunderlag: pinnad revision, funktion/bank/asset, mätpunkt
Berörda moduler och riktig konsument:
Input och startstate (ROM, save, RTC, karta, flags, RNG):
Output och stateändringar:
Timing (inputlatch/yield/resume/presentation) och RNG-anrop:
Persistens (mutation/checksumma/slot/dirty revision):
Fel/gränsfall och avbrott:
Acceptansfall: normalt, negativt, gräns, återinträde/omstart
Testnivåer och befintliga kommandon som återanvänds:
Ny kunskapslucka → separat mät-ID:
Bevis och integrerad revision när färdigt:
```

Detta är även krav på 6–9:s senare kort. Inventeringspaket är inte redo att
implementeras innan den första konkreta konsumenten, originalkontraktet och
acceptansfallet har specificerats. Varje liten leverans uppdaterar PLAN:s
aktuella status; stora framtidsrubriker blir inte automatiskt klara.
