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

  /* ---------------- recherche avancée ---------------- */

  /// Sans accents ni majuscules, comme la recherche simple.
  function fold(text) {
    return String(text || "")
      .toLowerCase()
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "");
  }

  /// Noms de champs acceptés (sans accents) → champ.
  const FIELDS = {
    titre: "title", title: "title", t: "title",
    artiste: "artist", artist: "artist", a: "artist", par: "artist",
    album: "album", al: "album",
    annee: "year", an: "year", year: "year",
    genre: "genre", g: "genre",
    duree: "duration", duration: "duration", d: "duration",
    format: "format", ext: "format",
    ecoutes: "plays", ecoute: "plays", plays: "plays",
    ajoute: "added", added: "added",
    est: "is", is: "is",
  };
  const IS = {
    favori: "fav", favoris: "fav", fav: "fav", aime: "fav",
    clip: "clip", clips: "clip", video: "clip",
    muet: "mute",
    jamais: "never", nouveau: "never",
    ecoute: "played", joue: "played",
  };

  /// Découpe la saisie : les guillemets gardent les espaces (artiste:"Ninho Niska").
  function tokens(text) {
    const out = [];
    let cur = "";
    let quoted = false;
    for (const ch of String(text || "")) {
      if (ch === '"') {
        quoted = !quoted;
        cur += ch;
      } else if (/\s/.test(ch) && !quoted) {
        if (cur) out.push(cur);
        cur = "";
      } else {
        cur += ch;
      }
    }
    if (cur) out.push(cur);
    return out;
  }

  const unquote = (v) => v.replace(/"/g, "").trim();

  /// Durée en secondes : « 3:30 », « 3m », « 210s », « 3,5 » (minutes).
  function seconds(v) {
    const t = v.trim().replace(",", ".");
    let m = t.match(/^(\d+):(\d{1,2})$/);
    if (m) return Number(m[1]) * 60 + Number(m[2]);
    m = t.match(/^(\d+(?:\.\d+)?)\s*(s|sec|m|min|mn|h)?$/);
    if (!m) return NaN;
    const n = Number(m[1]);
    const unit = m[2] || "m";
    return unit === "h" ? n * 3600 : unit.startsWith("s") ? n : n * 60;
  }

  /// Ancienneté en jours : « 30j », « 2sem », « 3mois », « 1an », « 10 » (jours).
  function days(v) {
    const m = fold(v).trim().replace(",", ".").match(/^(\d+(?:\.\d+)?)\s*(j|jours?|d|sem|semaines?|w|mois|m|ans?|y)?$/);
    if (!m) return NaN;
    const n = Number(m[1]);
    const u = m[2] || "j";
    if (/^(sem|w)/.test(u)) return n * 7;
    if (u === "mois" || u === "m") return n * 30;
    if (/^(an|y)/.test(u)) return n * 365;
    return n;
  }

  /// Valeur numérique ou intervalle (« 2010-2015 », « 3:00-4:30 »).
  function numbers(v, parse) {
    const range = v.match(/^([^-]+?)\s*(?:-|\.\.)\s*([^-]+)$/);
    if (range) {
      const lo = parse(range[1]);
      const hi = parse(range[2]);
      if (isFinite(lo) && isFinite(hi)) return { lo: Math.min(lo, hi), hi: Math.max(lo, hi) };
      return null;
    }
    const n = parse(v);
    return isFinite(n) ? { n } : null;
  }

  /// Analyse une recherche : mots libres, mots exclus (« -live ») et filtres par champ
  /// (« artiste:ninho », « année>2018 », « durée<3:00 », « est:favori »). Un filtre
  /// incompris reste un mot libre.
  function parseQuery(text) {
    const q = { words: [], exclude: [], terms: [], advanced: false };
    for (const raw of tokens(text)) {
      let tok = raw;
      let neg = false;
      if (tok.length > 1 && tok[0] === "-") {
        neg = true;
        tok = tok.slice(1);
      }
      const m = tok.match(/^([^\s:<>="]+?)(>=|<=|:|>|<|=)(.+)$/);
      const field = m ? FIELDS[fold(m[1])] : null;
      let term = null;
      if (field) {
        const op = m[2];
        const value = unquote(m[3]);
        if (field === "is") {
          const flag = IS[fold(value)];
          if (flag) term = { field, flag };
        } else if (field === "year") {
          const v = numbers(value, (x) => (/^\d{4}$/.test(x.trim()) ? Number(x) : NaN));
          if (v) term = { field, op, ...v };
        } else if (field === "duration") {
          const v = numbers(value, seconds);
          if (v) term = { field, op, ...v };
        } else if (field === "plays") {
          const v = numbers(value, (x) => (/^\d+$/.test(x.trim()) ? Number(x) : NaN));
          if (v) term = { field, op, ...v };
        } else if (field === "added") {
          const n = days(value);
          if (isFinite(n)) term = { field, op, n };
        } else if (value) {
          term = { field, op, text: fold(value) };
        }
      }
      if (term) {
        term.neg = neg;
        q.terms.push(term);
        q.advanced = true;
      } else {
        const word = fold(unquote(tok));
        if (!word) continue;
        if (neg) {
          q.exclude.push(word);
          q.advanced = true;
        } else {
          q.words.push(word);
        }
      }
    }
    return q;
  }

  function compare(term, v) {
    if (term.lo !== undefined) return v >= term.lo && v <= term.hi;
    const n = term.n;
    switch (term.op) {
      case ">": return v > n;
      case ">=": return v >= n;
      case "<": return v < n;
      case "<=": return v <= n;
      default:
        // « durée:3:30 » : à 5 s près ; les autres champs, la valeur exacte.
        return term.field === "duration" ? Math.abs(v - n) <= 5 : v === n;
    }
  }

  function textMatch(term, value) {
    const v = fold(value);
    return term.op === "=" ? v === term.text : v.includes(term.text);
  }

  function test(term, t, ctx) {
    switch (term.field) {
      case "title": return textMatch(term, t.title);
      case "album": return textMatch(term, t.album);
      case "genre": return textMatch(term, t.genre);
      case "artist":
        return [t.artist, t.album_artist, ...(t.artists || []), ...(t.featured || [])].some((a) => a && textMatch(term, a));
      case "format": {
        const ext = fold((String(t.path || "").match(/\.([a-z0-9]+)$/i) || [])[1]);
        return ext === term.text.replace(/^\./, "");
      }
      case "year": return t.year > 0 && compare(term, t.year);
      case "duration": return t.duration_ms > 0 && compare(term, t.duration_ms / 1000);
      case "plays": return compare(term, (ctx.plays[t.id] && ctx.plays[t.id].count) || 0);
      case "added": {
        if (!t.mtime) return false;
        const age = (ctx.now / 1000 - t.mtime) / 86400;
        // « ajouté<30j » ou « ajouté:30j » : ces 30 derniers jours ; « ajouté>1an » : plus vieux.
        return term.op === ">" || term.op === ">=" ? age > term.n : age <= term.n;
      }
      case "is":
        switch (term.flag) {
          case "fav": return ctx.favorites.has(t.id);
          case "clip": return !!t.video;
          case "mute": return t.has_audio === false;
          case "never": return !(ctx.plays[t.id] && ctx.plays[t.id].count > 0);
          case "played": return !!(ctx.plays[t.id] && ctx.plays[t.id].count > 0);
        }
    }
    return true;
  }

  /// Vrai si le morceau répond à la recherche. ctx = { key(t) : texte de recherche
  /// (titre, artiste, album, sans accents), plays, favorites (Set), now (ms) }.
  function matchQuery(q, t, ctx) {
    if (q.words.length || q.exclude.length) {
      const key = ctx.key(t);
      for (const w of q.words) if (!key.includes(w)) return false;
      for (const w of q.exclude) if (key.includes(w)) return false;
    }
    for (const term of q.terms) if (test(term, t, ctx) === term.neg) return false;
    return true;
  }

  const api = { freshBag, nextIndex, prevIndex, syncFolders, lyricTime, fileTime, addAnchor, parseQuery, matchQuery };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.EchoLogic = api;
})(typeof window !== "undefined" ? window : globalThis);
