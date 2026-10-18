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

  /// Calage actif à `atMs` dans le fichier : le dernier passé, sinon le premier.
  function activeAnchor(anchors, atMs) {
    if (!anchors || !anchors.length) return null;
    let current = null;
    for (const a of anchors) if (a.at_ms <= atMs && (!current || a.at_ms > current.at_ms)) current = a;
    return current || anchors.reduce((m, a) => (a.at_ms < m.at_ms ? a : m));
  }

  /// Moment des paroles qui correspond à `atMs` dans le fichier. Chaque calage vaut
  /// jusqu'au suivant : un clip coupé par une scène se recale en deux clics.
  function lyricTime(anchors, atMs) {
    const a = activeAnchor(anchors, atMs);
    return a ? atMs - a.at_ms + a.lyric_ms : atMs;
  }

  /// Inverse : où aller dans le fichier pour entendre `lyricMs`, en restant dans le
  /// passage en cours si la ligne y est (un clip peut rejouer le refrain plus loin).
  function fileTime(anchors, lyricMs, nowMs) {
    if (!anchors || !anchors.length) return lyricMs;
    const sorted = [...anchors].sort((x, y) => x.at_ms - y.at_ms);
    const cur = activeAnchor(sorted, nowMs);
    const i = sorted.indexOf(cur);
    const end = i + 1 < sorted.length ? sorted[i + 1].at_ms : Infinity;
    const inCurrent = cur.at_ms + (lyricMs - cur.lyric_ms);
    if (lyricMs >= cur.lyric_ms && inCurrent < end) return inCurrent;
    const other = sorted.find((a) => lyricMs >= a.lyric_ms) || sorted[0];
    return Math.max(0, other.at_ms + (lyricMs - other.lyric_ms));
  }

  /// Ajoute (ou remplace) un calage : à moins de 2 s d'un autre, c'est une correction.
  function addAnchor(anchors, atMs, lyricMs) {
    const kept = (anchors || []).filter((a) => Math.abs(a.at_ms - atMs) > 2000);
    kept.push({ at_ms: Math.max(0, Math.round(atMs)), lyric_ms: Math.max(0, Math.round(lyricMs)) });
    return kept.sort((x, y) => x.at_ms - y.at_ms);
  }

  const api = { freshBag, nextIndex, prevIndex, syncFolders, lyricTime, fileTime, addAnchor };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.EchoLogic = api;
})(typeof window !== "undefined" ? window : globalThis);
