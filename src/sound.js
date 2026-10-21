/* ============================================================
   Écho — moteur audio
   Deux lecteurs (pour le fondu enchaîné) qui passent par une même chaîne :
   égaliseur 5 bandes → volume égal → limiteur → volume général.
   La chaîne Web Audio n'est montée que si un réglage de son en a besoin.
   Sous Linux, c'est le cœur de l'app qui lit la musique (GStreamer) : `useNative`
   remplace alors les deux lecteurs par une télécommande, et ce module ne fait plus
   que transmettre le volume.
   ============================================================ */

const Sound = (() => {
  const players = [document.getElementById("audio"), document.getElementById("audio2")];
  let active = 0;

  /// Préréglages de l'égaliseur, en dB, pour 60 Hz, 250 Hz, 1 kHz, 4 kHz, 12 kHz.
  const EQ_PRESETS = {
    flat: [0, 0, 0, 0, 0],
    bass: [6, 3.5, 0, 0, 1],
    vocal: [-2, -1, 2.5, 3.5, 1],
    treble: [-1, 0, 0, 2.5, 5],
    rap: [6.5, 3, -1.5, 1.5, 3],
    acoustic: [2.5, 1, 1, 2, 3],
  };
  const BANDS = [60, 250, 1000, 4000, 12000];

  /// Volume égal : niveau visé, proche des −14 LUFS des plateformes de streaming
  /// (Spotify, YouTube), et bornes de correction (−14 dB à +15 dB) pour ne pas
  /// faire ressortir le souffle d'un passage très calme.
  const TARGET_RMS = 0.18;
  const MIN_GAIN = 0.2;
  const MAX_GAIN = 5.6;

  let ctx = null;
  let sources = []; // gain propre à chaque lecteur (pour les fondus)
  let filters = [];
  let level = null; // correction du volume égal
  let master = null; // volume général
  let analyser = null;
  let freq = null; // mesures du spectre (visualiseur)
  let levelTimer = null;

  let volume = 0.9;
  let muted = false;
  let eq = "flat";
  let loudness = false;
  let trackId = null;
  let measured = 0; // énergie moyenne mesurée sur le morceau en cours
  let samples = 0; // nombre de mesures prises sur ce morceau
  const gains = loadGains();

  function loadGains() {
    try {
      return new Map(JSON.parse(localStorage.getItem("echo.gains") || "[]"));
    } catch {
      return new Map();
    }
  }
  function saveGains() {
    try {
      localStorage.setItem("echo.gains", JSON.stringify([...gains].slice(-3000)));
    } catch {
      /* stockage indisponible : la correction repart de zéro, sans gravité */
    }
  }

  const needsGraph = () => loudness || eq !== "flat";

  /// Monte la chaîne Web Audio (une seule fois : un lecteur ne peut plus s'en détacher).
  function graph() {
    if (ctx) return true;
    const Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) return false;
    try {
      ctx = new Ctx({ latencyHint: "playback" });
      filters = BANDS.map((freq, i) => {
        const f = ctx.createBiquadFilter();
        f.type = i === 0 ? "lowshelf" : i === BANDS.length - 1 ? "highshelf" : "peaking";
        f.frequency.value = freq;
        f.Q.value = 0.9;
        return f;
      });
      for (let i = 0; i < filters.length - 1; i++) filters[i].connect(filters[i + 1]);
      analyser = ctx.createAnalyser();
      analyser.fftSize = 2048;
      level = ctx.createGain();
      // Limiteur : rattrape les crêtes qu'un égaliseur ou une correction feraient saturer.
      const limiter = ctx.createDynamicsCompressor();
      limiter.threshold.value = -1.5;
      limiter.knee.value = 0;
      limiter.ratio.value = 20;
      limiter.attack.value = 0.003;
      limiter.release.value = 0.2;
      master = ctx.createGain();
      const last = filters[filters.length - 1];
      last.connect(analyser);
      last.connect(level);
      level.connect(limiter);
      limiter.connect(master);
      master.connect(ctx.destination);
      sources = players.map((p) => {
        const node = ctx.createMediaElementSource(p);
        const g = ctx.createGain();
        node.connect(g);
        g.connect(filters[0]);
        return g;
      });
      players.forEach((p, i) => {
        p.volume = 1;
        sources[i].gain.value = i === active ? 1 : 0;
      });
      applyEq();
      applyVolume();
      return true;
    } catch (err) {
      console.error("chaîne audio indisponible", err);
      ctx = null;
      return false;
    }
  }

  function applyEq() {
    if (!ctx) return;
    const values = EQ_PRESETS[eq] || EQ_PRESETS.flat;
    filters.forEach((f, i) => f.gain.setTargetAtTime(values[i], ctx.currentTime, 0.05));
    // L'égaliseur ajoute du volume : on le compense pour ne pas saturer.
    const boost = Math.max(0, ...values);
    if (!loudness) level.gain.setTargetAtTime(Math.pow(10, -boost / 20 / 1.5), ctx.currentTime, 0.1);
  }

  function applyVolume() {
    const v = muted ? 0 : volume;
    if (ctx) {
      master.gain.setTargetAtTime(v, ctx.currentTime, 0.03);
    } else {
      players.forEach((p) => (p.volume = v));
    }
  }

  /// Volume égal : mesure le niveau en continu et ajuste doucement la correction.
  function tickLevel() {
    // Pendant un fondu, on entend encore le morceau précédent : la mesure serait faussée.
    if (!ctx || !loudness || players[active].paused || fadeTimer) return;
    const data = new Float32Array(analyser.fftSize);
    analyser.getFloatTimeDomainData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
    const rms = Math.sqrt(sum / data.length);
    if (rms < 0.004) return; // silence : on ne corrige pas sur du vide
    // Moyenne exacte sur les premières secondes (réglage rapide), puis lissée.
    samples += 1;
    const weight = Math.max(0.03, 1 / samples);
    measured = measured ? measured * (1 - weight) + rms * weight : rms;
    const wanted = Math.min(MAX_GAIN, Math.max(MIN_GAIN, TARGET_RMS / measured));
    level.gain.setTargetAtTime(wanted, ctx.currentTime, samples < 12 ? 0.4 : 1.2);
    if (trackId) gains.set(trackId, wanted);
  }

  function startLevel() {
    clearInterval(levelTimer);
    if (ctx && loudness) levelTimer = setInterval(tickLevel, 250);
  }

  /// Nouveau morceau : on reprend la correction déjà apprise pour lui.
  function beginTrack(id) {
    if (trackId && gains.has(trackId)) saveGains();
    trackId = id;
    measured = 0;
    samples = 0;
    if (ctx && loudness) {
      const known = gains.get(id);
      level.gain.setTargetAtTime(known || 1, ctx.currentTime, 0.05);
      // Correction déjà apprise : elle compte comme quelques secondes de mesure.
      if (known) {
        measured = TARGET_RMS / known;
        samples = 12;
      }
    }
  }

  let fadeTimer = null;
  /// Lecteur du cœur : { media, setVolume(volume, muted) } ; null, la page lit elle-même.
  let native = null;

  return {
    get element() {
      return native ? native.media : players[active];
    },
    get other() {
      return native ? native.media : players[1 - active];
    },
    players,

    /// Spectre du son pour le visualiseur : `out` reçoit des bandes de 0 à 1, de 40 Hz
    /// à 16 kHz (échelle logarithmique, comme celles du lecteur du cœur). Faux sans
    /// chaîne Web Audio.
    spectrum(out) {
      if (native || !graph() || !analyser) return false;
      if (!freq || freq.length !== analyser.frequencyBinCount) freq = new Float32Array(analyser.frequencyBinCount);
      analyser.getFloatFrequencyData(freq);
      const hz = ctx.sampleRate / 2 / freq.length;
      const n = out.length;
      const ratio = Math.pow(16000 / 40, 1 / n);
      const floor = -75;
      for (let i = 0; i < n; i++) {
        const f0 = 40 * Math.pow(ratio, i);
        const a = Math.min(freq.length - 1, Math.floor(f0 / hz));
        const b = Math.min(freq.length, Math.max(a + 1, Math.ceil((f0 * ratio) / hz)));
        let db = floor;
        for (let k = a; k < b; k++) db = Math.max(db, freq[k]);
        out[i] = Math.min(1, Math.max(0, (db + (6 * i) / n - floor) / -floor));
      }
      return true;
    },

    /// La musique passe par le cœur : plus de chaîne Web Audio, plus de lecteurs ici.
    useNative(bridge) {
      native = bridge;
      players.forEach((p) => {
        p.pause();
        p.removeAttribute("src");
      });
      clearInterval(levelTimer);
      // La chaîne Web Audio a pu être montée avant de savoir : elle tiendrait un flux
      // son ouvert (et sa mémoire) pour rien.
      if (ctx) {
        ctx.close().catch(() => {});
        ctx = null;
      }
    },

    /// Applique les réglages de son (au démarrage et à chaque changement).
    configure(settings) {
      if (native) {
        // L'égaliseur, le volume égal et le fondu sont lus par le cœur dans les réglages.
        volume = Math.min(1, Math.max(0, settings.volume / 100));
        native.setVolume(volume, muted);
        return;
      }
      eq = EQ_PRESETS[settings.eq] ? settings.eq : "flat";
      loudness = !!settings.loudness;
      volume = Math.min(1, Math.max(0, settings.volume / 100));
      if (needsGraph()) graph();
      if (ctx) {
        applyEq();
        if (!loudness) clearInterval(levelTimer);
        else startLevel();
      }
      applyVolume();
    },
    /// Volume et son coupé tels que le cœur les a (page rouverte) : rien n'est renvoyé.
    adopt(v, m) {
      volume = Math.min(1, Math.max(0, v));
      muted = !!m;
    },
    setVolume(v) {
      volume = Math.min(1, Math.max(0, v));
      if (native) native.setVolume(volume, muted);
      else applyVolume();
    },
    setMuted(m) {
      muted = !!m;
      if (native) native.setVolume(volume, muted);
      else applyVolume();
    },
    get muted() {
      return muted;
    },

    /// À appeler juste avant de lancer la lecture : la chaîne doit être réveillée.
    resume() {
      if (native) return;
      if (ctx && ctx.state === "suspended") ctx.resume().catch(() => {});
      startLevel();
    },
    beginTrack,

    /// Arrête un fondu en cours : le lecteur sortant se tait tout de suite.
    cancelFade() {
      if (native || !fadeTimer) return;
      clearInterval(fadeTimer);
      fadeTimer = null;
      const out = players[1 - active];
      out.pause();
      out.removeAttribute("src");
      out.load();
      if (ctx) {
        sources[1 - active].gain.cancelScheduledValues(ctx.currentTime);
        sources[1 - active].gain.value = 0;
        sources[active].gain.cancelScheduledValues(ctx.currentTime);
        sources[active].gain.value = 1;
      } else {
        players[active].volume = muted ? 0 : volume;
      }
    },
    get fading() {
      return !!fadeTimer;
    },

    /// Fondu enchaîné : le lecteur inactif (déjà chargé) monte pendant que l'actif descend.
    /// Le lecteur entrant devient l'actif tout de suite.
    crossfade(seconds) {
      this.cancelFade();
      const out = active;
      active = 1 - active;
      const incoming = players[active];
      const outgoing = players[out];
      const ms = Math.max(500, seconds * 1000);
      if (ctx) {
        const t = ctx.currentTime;
        sources[active].gain.cancelScheduledValues(t);
        sources[active].gain.setValueAtTime(0, t);
        sources[active].gain.linearRampToValueAtTime(1, t + ms / 1000);
        sources[out].gain.cancelScheduledValues(t);
        sources[out].gain.setValueAtTime(sources[out].gain.value, t);
        sources[out].gain.linearRampToValueAtTime(0, t + ms / 1000);
      } else {
        incoming.volume = 0;
      }
      const started = Date.now();
      fadeTimer = setInterval(() => {
        const k = Math.min(1, (Date.now() - started) / ms);
        if (!ctx) {
          const v = muted ? 0 : volume;
          incoming.volume = v * k;
          outgoing.volume = v * (1 - k);
        }
        if (k >= 1) {
          clearInterval(fadeTimer);
          fadeTimer = null;
          outgoing.pause();
          outgoing.removeAttribute("src");
          outgoing.load();
        }
      }, 50);
      return incoming;
    },
  };
})();
