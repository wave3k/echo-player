// Règles de lecture d'Écho. Lancer : node --test tests/
const test = require("node:test");
const assert = require("node:assert/strict");
const { freshBag, nextIndex, prevIndex, syncFolders } = require("../src/logic.js");

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
