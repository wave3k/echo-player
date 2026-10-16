/* ============================================================
   Écho — règles de lecture, sans rien d'affiché.
   Gardées à part pour être testées (node --test tests/).
   ============================================================ */

(function (root) {
  /// Mémoire de l'aléatoire pour une file : morceaux déjà joués et ordre de passage.
  function freshBag(queue) {
    return { queue, played: new Set(), history: [] };
  }

  /// Le morceau suivant. `auto` : la fin du morceau, pas un clic sur Suivant.
  /// ctx = { n, index, shuffle, repeat, bag } ; renvoie -1 pour s'arrêter.
  function nextIndex(ctx, auto, rand = Math.random) {
    const { n, index, shuffle, repeat, bag } = ctx;
    if (n === 0) return -1;
    // « Un titre » rejoue le même morceau à la fin, aléatoire ou non.
    if (auto && repeat === "one") return index;
    if (shuffle && n > 1) {
      // Chaque morceau passe une fois avant qu'un autre tour commence.
      bag.played.add(index);
      let pool = [];
      for (let i = 0; i < n; i++) if (!bag.played.has(i)) pool.push(i);
      if (pool.length === 0) {
        if (auto && repeat === "off") return -1;
        bag.played = new Set([index]);
        for (let i = 0; i < n; i++) if (i !== index) pool.push(i);
      }
      return pool[Math.floor(rand() * pool.length)];
    }
    if (index + 1 < n) return index + 1;
    if (auto && repeat === "all") return 0;
    if (auto) return -1;
    return 0;
  }

  /// Le morceau précédent : en aléatoire, celui qui a vraiment joué avant.
  function prevIndex(ctx) {
    const { n, index, shuffle, bag } = ctx;
    if (n === 0) return -1;
    if (shuffle && bag.history.length) {
      const back = bag.history.pop();
      bag.played.delete(index);
      if (back < n) return back;
    }
    return index - 1 < 0 ? n - 1 : index - 1;
  }

  /// Les dossiers appartiennent au cœur : la copie locale des réglages suit chaque
  /// réponse, sinon le prochain enregistrement renverrait une liste périmée.
  function syncFolders(settings, snapshot) {
    if (settings && snapshot && Array.isArray(snapshot.folders)) settings.folders = [...snapshot.folders];
    return settings;
  }

  const api = { freshBag, nextIndex, prevIndex, syncFolders };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.EchoLogic = api;
})(typeof window !== "undefined" ? window : globalThis);
