// Règles de lecture d'Écho. Lancer : node --test tests/*.test.js
const test = require("node:test");
const assert = require("node:assert/strict");
const { freshBag, nextIndex, prevIndex, syncFolders, lyricTime, fileTime, addAnchor } = require("../src/logic.js");

const ctx = (over = {}) => ({ n: 5, index: 0, shuffle: false, repeat: "off", bag: freshBag([]), ...over });

test("répéter un titre rejoue le même morceau, même au milieu de la file", () => {
  assert.equal(nextIndex(ctx({ index: 0, repeat: "one" }), true), 0);
  assert.equal(nextIndex(ctx({ index: 2, repeat: "one" }), true), 2);
});

test("répéter un titre passe avant l'aléatoire", () => {
  assert.equal(nextIndex(ctx({ index: 3, repeat: "one", shuffle: true }), true), 3);
});

test("Suivant à la main avance, même en répétition d'un titre", () => {
  assert.equal(nextIndex(ctx({ index: 1, repeat: "one" }), false), 2);
});

test("fin de file : arrêt sans répétition, retour au début sinon", () => {
  assert.equal(nextIndex(ctx({ index: 4 }), true), -1);
  assert.equal(nextIndex(ctx({ index: 4, repeat: "all" }), true), 0);
  assert.equal(nextIndex(ctx({ index: 4 }), false), 0);
});

test("aléatoire : chaque morceau passe une fois avant qu'un autre tour commence", () => {
  const c = ctx({ shuffle: true, repeat: "all" });
  const seen = [c.index];
  for (let k = 0; k < 4; k++) {
    c.index = nextIndex(c, true);
    seen.push(c.index);
  }
  assert.deepEqual([...seen].sort(), [0, 1, 2, 3, 4]);
  // Tour suivant : jamais deux fois le même d'affilée.
  const after = nextIndex(c, true);
  assert.notEqual(after, c.index);
});

test("aléatoire sans répétition : s'arrête une fois tout joué", () => {
  const c = ctx({ n: 3, shuffle: true });
  c.index = nextIndex(c, true);
  c.index = nextIndex(c, true);
  assert.equal(nextIndex(c, true), -1);
});

test("aléatoire : Précédent ramène au morceau joué juste avant", () => {
  const c = ctx({ index: 0, shuffle: true });
  const order = [0];
  for (let k = 0; k < 3; k++) {
    const next = nextIndex(c, false);
    c.bag.history.push(c.index);
    c.index = next;
    order.push(next);
  }
  assert.equal(prevIndex(c), order[2]);
});

test("Précédent sans aléatoire recule, et boucle au début", () => {
  assert.equal(prevIndex(ctx({ index: 2 })), 1);
  assert.equal(prevIndex(ctx({ index: 0 })), 4);
});

test("la copie locale des réglages suit les dossiers du cœur", () => {
  const settings = { folders: ["/musique"], volume: 70 };
  syncFolders(settings, { folders: ["/musique", "/nouveau"] });
  assert.deepEqual(settings.folders, ["/musique", "/nouveau"]);
  assert.equal(settings.volume, 70);
});

// God's Plan : dans le clip, le son démarre après 36 s de scène, puis reprend du début à 119,7 s.
const godsPlan = [
  { at_ms: 48900, lyric_ms: 12510 },
  { at_ms: 119700, lyric_ms: 12510 },
];

test("paroles : sans calage, le fichier et les paroles avancent ensemble", () => {
  assert.equal(lyricTime([], 5000), 5000);
  assert.equal(fileTime([], 5000, 0), 5000);
});

test("paroles : chaque calage vaut jusqu'au suivant", () => {
  assert.equal(lyricTime(godsPlan, 60000), 23610);
  assert.equal(lyricTime(godsPlan, 119700), 12510);
  assert.equal(lyricTime(godsPlan, 130000), 22810);
  assert.ok(lyricTime(godsPlan, 10000) < 0, "l'intro du clip tombe avant les paroles");
});

test("paroles : un clic sur une ligne reste dans le passage en cours", () => {
  // On écoute la 2e partie : la ligne à 20 s des paroles est jouée à 127,19 s, pas à 56,39 s.
  assert.equal(fileTime(godsPlan, 20000, 125000), 127190);
  assert.equal(fileTime(godsPlan, 20000, 60000), 56390);
});

test("paroles : recaler près d'un calage le remplace", () => {
  const a = addAnchor(godsPlan, 49500, 12510);
  assert.equal(a.length, 2);
  assert.equal(a[0].at_ms, 49500);
  assert.equal(addAnchor(godsPlan, 90000, 50000).length, 3);
});


test("le texte inséré dans le HTML est échappé, apostrophe comprise", () => {
  const { escapeHtml } = require("../src/logic.js");
  assert.equal(escapeHtml(`<b a="x" b='y'>&</b>`), "&lt;b a=&quot;x&quot; b=&#39;y&#39;&gt;&amp;&lt;/b&gt;");
  assert.equal(escapeHtml(null), "");
  assert.equal(escapeHtml(42), "42");
});

/* ---------------- recherche avancée ---------------- */
const { parseQuery, matchQuery } = require("../src/logic.js");

const NOW = Date.UTC(2026, 9, 21);
const lib = [
  { id: "1", path: "/m/Ninho - Lettre.flac", title: "Lettre à une femme", artist: "Ninho", album: "Destin", year: 2019, duration_ms: 214000, mtime: NOW / 1000 - 3 * 86400, genre: "Rap" },
  { id: "2", path: "/m/Tiakola x Dave - Meridian.webm", title: "Meridian", artist: "Tiakola x Dave", artists: ["Tiakola", "Dave"], album: "", year: 0, duration_ms: 255000, mtime: NOW / 1000 - 400 * 86400, video: true },
  { id: "3", path: "/m/Niska - Réseaux (Live).mp3", title: "Réseaux (Live)", artist: "Niska", featured: ["Ninho"], album: "Commando", year: 2017, duration_ms: 170000, mtime: NOW / 1000 - 40 * 86400 },
];
const searchCtx = {
  key: (t) => `${t.title}\u0001${t.artist}\u0001${t.album}`.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, ""),
  plays: { 1: { count: 12 }, 3: { count: 1 } },
  favorites: new Set(["2"]),
  now: NOW,
};
const find = (text) => lib.filter((t) => matchQuery(parseQuery(text), t, searchCtx)).map((t) => t.id);

test("recherche : mots libres sans accents, et mots exclus", () => {
  assert.deepEqual(find("reseaux"), ["3"]);
  assert.deepEqual(find("-live"), ["1", "2"]);
  assert.equal(parseQuery("ninho").advanced, false);
  assert.equal(parseQuery("-live").advanced, true);
});

test("recherche : champs de texte, featurings compris, et guillemets", () => {
  assert.deepEqual(find("artiste:ninho"), ["1", "3"]);
  assert.deepEqual(find("artiste=ninho"), ["1", "3"]);
  assert.deepEqual(find("artiste:dave"), ["2"]);
  assert.deepEqual(find('titre:"lettre à"'), ["1"]);
  assert.deepEqual(find("album:commando"), ["3"]);
  assert.deepEqual(find("genre:rap"), ["1"]);
  assert.deepEqual(find("-artiste:ninho"), ["2"]);
});

test("recherche : années, durées, écoutes, formats", () => {
  assert.deepEqual(find("année>2018"), ["1"]);
  assert.deepEqual(find("annee:2015-2018"), ["3"]);
  assert.deepEqual(find("durée<3:00"), ["3"]);
  assert.deepEqual(find("duree>4m"), ["2"]);
  assert.deepEqual(find("durée:3:34"), ["1"]);
  assert.deepEqual(find("écoutes>=1"), ["1", "3"]);
  assert.deepEqual(find("ecoutes>10"), ["1"]);
  assert.deepEqual(find("format:flac"), ["1"]);
});

test("recherche : ajoutés récemment et drapeaux", () => {
  assert.deepEqual(find("ajouté<30j"), ["1"]);
  assert.deepEqual(find("ajoute:2mois"), ["1", "3"]);
  assert.deepEqual(find("ajouté>1an"), ["2"]);
  assert.deepEqual(find("est:favori"), ["2"]);
  assert.deepEqual(find("est:clip"), ["2"]);
  assert.deepEqual(find("est:jamais"), ["2"]);
  assert.deepEqual(find("-est:clip artiste:ninho"), ["1", "3"]);
});

test("recherche : un filtre incompris reste un mot libre", () => {
  const q = parseQuery("foo:bar année>abc");
  assert.equal(q.terms.length, 0);
  assert.deepEqual(q.words, ["foo:bar", "annee>abc"]);
});
