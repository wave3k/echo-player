/* ============================================================
   Écho — logique de l'application
   ============================================================ */

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { open: openDialog, save: saveDialog } = window.__TAURI__.dialog;
const { listen } = window.__TAURI__.event;

const ROW_HEIGHT = 52;

const state = {
  settings: null,
  folders: [],
  foldersMissing: [],
  tracks: [],
  formats: [],
  cacheBytes: 0,
  view: "listen",
  queue: [],
  queueIndex: -1,
  search: "",
  minSearch: "",
  sort: "title",
  minimalAlbum: null,
  seeking: false,
  resuming: null,
  shuffle: false,
  repeat: "off",
};

const el = (id) => document.getElementById(id);

/* ---------------- petits outils ---------------- */

function normalize(text) {
  return (text || "")
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "");
}

function formatTime(ms) {
  if (!ms || ms < 0) return "0:00";
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${ss}`;
  return `${m}:${ss}`;
}

function hueFor(text) {
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (h * 31 + text.charCodeAt(i)) % 360;
  return h;
}

function escapeHtml(text) {
  return String(text ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

/// Table id → morceau, refaite à chaque nouvelle bibliothèque.
let trackIndex = new Map();
function trackById(id) {
  return trackIndex.get(id) || null;
}

/// Tri « à la française », réutilisé : bien plus rapide que localeCompare à chaque comparaison.
const COLLATOR = new Intl.Collator("fr", { sensitivity: "base" });

/// Texte de recherche d'un morceau, préparé une fois (sans accents, en minuscules).
function searchKey(track) {
  if (track._q === undefined) track._q = normalize(`${track.title}\u0001${track.artist}\u0001${track.album}`);
  return track._q;
}
function currentId() {
  return state.queue[state.queueIndex] || null;
}
function currentTrack() {
  return trackById(currentId());
}
function audioTracks() {
  return state.tracks;
}

function mediaEl() {
  return audio;
}

/* ---------------- icônes écrites dans le HTML ---------------- */

function applyIcons(root = document) {
  root.querySelectorAll("[data-icon]").forEach((node) => {
    const name = node.dataset.icon;
    if (!ICONS[name]) return;
    node.innerHTML = ICONS[name];
    const size = node.dataset.size;
    if (size) {
      node.style.width = `${size}px`;
      node.style.height = `${size}px`;
    }
    node.classList.add("ic");
  });
}

/* ---------------- pochettes ---------------- */

/// Adresse (protocole d'asset) de chaque pochette déjà connue. Le navigateur gère
/// lui-même le cache des images : rien n'est recopié en mémoire ici.
const coverUrls = new Map();
/// Vignette de 128 px de chaque pochette, pour les petits affichages (listes, mini-lecteur).
const thumbUrls = new Map();
const coverPending = new Map();

function rememberCover(id, paths) {
  coverUrls.set(id, paths ? convertFileSrc(paths.full) : null);
  if (paths && paths.thumb) thumbUrls.set(id, convertFileSrc(paths.thumb));
}

/// Petit affichage : la vignette suffit, l'image entière y serait décodée pour rien.
/// La plus grande dimension compte : une tranche haute et étroite a besoin de l'image entière.
function isSmallCover(node) {
  if (node.classList.contains("thumb")) return true;
  if (!node.dataset.coverSize) {
    const size = Math.max(node.offsetWidth, node.offsetHeight);
    if (!size) return false;
    node.dataset.coverSize = size * (window.devicePixelRatio || 1) <= 128 ? "s" : "l";
  }
  return node.dataset.coverSize === "s";
}

function coverSrc(id) {
  if (coverUrls.has(id)) return Promise.resolve(coverUrls.get(id));
  if (!coverPending.has(id)) {
    const request = invoke("get_cover_path", { id })
      .then((paths) => {
        rememberCover(id, paths);
        return coverUrls.get(id);
      })
      .catch(() => {
        // Adresse introuvable : la pochette est traitée comme absente (dégradé), pas en attente.
        rememberCover(id, null);
        return null;
      })
      .finally(() => coverPending.delete(id));
    coverPending.set(id, request);
  }
  return coverPending.get(id);
}

/// Pose la pochette (ou un dégradé) sur un élément, sans toucher à sa taille.
/// Sans pochette du tout, le dégradé porte le titre et l'artiste.
function paintCover(node, track, label = true) {
  const id = track && track.cover;
  const url = id ? (isSmallCover(node) && thumbUrls.get(id)) || coverUrls.get(id) : null;
  if (url) {
    node.style.background = `var(--cover-bg) url("${url}") center / cover no-repeat`;
    coverLabel(node, null);
    return;
  }
  // Pochette connue mais adresse pas encore arrivée : un fond neutre, pas le dégradé
  // « sans pochette » qui serait remplacé une fraction de seconde plus tard.
  if (id && !coverUrls.has(id)) {
    node.style.background = "var(--cover-bg)";
    coverLabel(node, null);
    return;
  }
  const hue = hueFor(track && track.id ? track.id : "x");
  node.style.background = `linear-gradient(140deg, hsl(${hue} 45% 30%), hsl(${(hue + 45) % 360} 45% 17%))`;
  coverLabel(node, label && track && !track.cover ? track : null);
}

/// Titre et artiste écrits sur une pochette vide (masqués sur les petites vignettes).
function coverLabel(node, track) {
  let label = node.querySelector(":scope > .cover-label");
  if (!track) {
    if (label) label.remove();
    return;
  }
  if (!label) {
    if (!node.dataset.positioned) {
      node.dataset.positioned = "1";
      if (getComputedStyle(node).position === "static") node.style.position = "relative";
    }
    label = document.createElement("span");
    label.className = "cover-label";
    label.setAttribute("aria-hidden", "true");
    label.append(document.createElement("b"), document.createElement("i"));
    node.appendChild(label);
  }
  label.firstChild.textContent = track.title || "Sans titre";
  label.lastChild.textContent = track.artist || "";
}

function noCover(node) {
  delete node.dataset.shown;
  delete node.dataset.coverOf;
  node.classList.add("no-cover");
  node.style.background = "";
  node.innerHTML = icon("note", 16);
}

/// `label` : faux pour une pochette d'album ou de playlist (le titre d'un morceau y tromperait).
function fillCover(node, track, label = true) {
  if (!node) return;
  if (!track || !state.settings.show_covers) {
    noCover(node);
    return;
  }
  // Déjà affichée telle quelle : la repeindre la ferait clignoter.
  // La vignette compte pour les petits affichages : quand elle arrive, elle remplace l'image entière.
  const thumb = node.classList.contains("thumb") || node.dataset.coverSize === "s" ? thumbUrls.has(track.cover) : 0;
  const shown = `${track.id}|${track.cover || ""}|${coverUrls.has(track.cover) ? 1 : 0}|${thumb ? 1 : 0}|${label}`;
  if (node.dataset.shown === shown && !node.classList.contains("no-cover")) return;
  node.dataset.shown = shown;
  // Pour `repaintCovers` : quel morceau cet élément montre.
  node.dataset.coverOf = track.id;
  node.dataset.coverLabel = label ? "1" : "0";
  node.classList.remove("no-cover");
  node.innerHTML = "";
  // Les petites vignettes des listes restent sans texte : illisible, et redessiné au défilement.
  if (node.classList.contains("thumb") || isSmallCover(node)) label = false;
  paintCover(node, track, label);

  if (!track.cover || coverUrls.has(track.cover)) return;
  // La pochette est reconnue : on la pose dès que son adresse est prête.
  const wanted = track.cover;
  coverSrc(wanted).then(() => {
    // L'élément montre peut-être déjà un autre morceau : on ne le repeint que s'il
    // attend toujours celui-ci.
    if (node.dataset.shown === shown) fillCover(node, track, label);
  });
}

/// Des pochettes sont arrivées (adresses, images de clips) : seules les pochettes
/// affichées qui changent vraiment sont repeintes, rien n'est reconstruit.
function repaintCovers() {
  document.querySelectorAll("[data-cover-of]").forEach((node) => {
    const track = trackById(node.dataset.coverOf);
    if (track) fillCover(node, track, node.dataset.coverLabel === "1");
  });
}

/* ============================================================
   Réglages
   ============================================================ */

const PALETTES = [
  { id: "nuit", name: "Nuit bleue", colors: ["#0d0d12", "#38bdf8", "#8b5cf6"] },
  { id: "aurore", name: "Aurore", colors: ["#131011", "#fb7185", "#fbbf24"] },
  { id: "menthe", name: "Menthe", colors: ["#0a1211", "#34d399", "#22d3ee"] },
  { id: "prune", name: "Prune", colors: ["#0f0b14", "#a78bfa", "#f472b6"] },
];

const FONTS = [
  { id: "space", name: "Space Grotesk", desc: "Par défaut · singulière, un peu rétro" },
  { id: "sora", name: "Sora", desc: "Large et nette, très lisible" },
  { id: "outfit", name: "Outfit", desc: "Douce et neutre" },
];

const MODES = [
  { id: "normal", name: "Normale" },
  { id: "compact", name: "Compacte" },
  { id: "minimal", name: "Minimaliste" },
];

const FORMAT_NAMES = { autres: "Autres" };

const EQ_CHOICES = [
  { value: "flat", label: "Neutre" },
  { value: "bass", label: "Basses" },
  { value: "rap", label: "Rap" },
  { value: "vocal", label: "Voix" },
  { value: "treble", label: "Aigus" },
  { value: "acoustic", label: "Acoustique" },
];

function systemPrefersDark() {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function resolvedTheme() {
  const t = state.settings.theme;
  if (t === "system") return systemPrefersDark() ? "dark" : "light";
  return t;
}

function applySettings() {
  const s = state.settings;
  const body = document.body;
  // Thème ou palette qui changent : tout bascule d'un coup, sans fondus décalés
  // (fonds instantanés, boutons en fondu : on verrait un éclair).
  if (body.dataset.theme !== resolvedTheme() || body.dataset.palette !== s.palette) {
    const root = document.documentElement;
    root.classList.add("switching");
    requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove("switching")));
  }
  body.dataset.theme = resolvedTheme();
  body.dataset.palette = s.palette;
  body.dataset.font = s.font;
  body.dataset.mode = s.mode;
  body.dataset.anim = s.animations ? "on" : "off";
  syncSettingsNav();
  document.documentElement.dataset.text = s.text_size;
  body.style.setProperty("--radius", `${s.radius}px`);
  body.style.setProperty("--radius-soft", `${(s.radius * 0.83).toFixed(1)}px`);
  body.style.setProperty("--radius-md", `${(s.radius * 1.17).toFixed(1)}px`);
  body.style.setProperty("--radius-lg", `${(s.radius * 1.5).toFixed(1)}px`);
  Sound.configure(s);
  const volume = el("volume");
  if (volume) {
    volume.value = String(s.volume);
    volume.style.setProperty("--p", `${s.volume}%`);
  }
}

let saveTimer = null;
function saveSettings(patch = {}, { refresh = true } = {}) {
  const modeChanged = patch.mode !== undefined && patch.mode !== state.settings.mode;
  Object.assign(state.settings, patch);
  applySettings();
  // Chaque disposition ne se dessine que quand elle est affichée.
  if (modeChanged) renderAll();
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      // Une bibliothèque ne revient que si elle a été relue (formats changés).
      const snap = await invoke("set_settings", { newSettings: state.settings });
      if (snap) applySnapshot(snap);
    } catch (err) {
      console.error("réglages non enregistrés", err);
    }
  }, 350);
  if (refresh) refreshSettingsPanel();
}

/* ---------------- fabrique de la page de réglages ---------------- */

function rowSwitch(label, desc, field, value, action = null, pending = false) {
  const attr = pending ? "" : action ? `data-action="${action}"` : `data-toggle="${field}"`;
  return `<div class="set-row">
    <div class="left">
      <div class="lab">${label}${pending ? ' <span class="badge">bientôt</span>' : ""}</div>
      <div class="desc">${desc}</div>
    </div>
    <div class="set-ctl"><button type="button" role="switch" aria-label="${label}" aria-checked="${value ? "true" : "false"}" class="switch${value ? " on" : ""}${pending ? " disabled" : ""}" ${attr}${pending ? " disabled" : ""}><i></i></button></div>
  </div>`;
}

function rowSeg(label, desc, field, options, value) {
  return `<div class="set-row">
    <div class="left"><div class="lab">${label}</div><div class="desc">${desc}</div></div>
    <div class="set-ctl"><span class="seg">${options
      .map((o) => `<button class="${String(o.value) === String(value) ? "on" : ""}" data-set="${field}" data-value="${o.value}">${o.label}</button>`)
      .join("")}</span></div>
  </div>`;
}

function rowButton(label, desc, buttons) {
  return `<div class="set-row">
    <div class="left"><div class="lab">${label}</div><div class="desc">${desc}</div></div>
    <div class="set-ctl">${buttons}</div>
  </div>`;
}

function sectionApparence() {
  const s = state.settings;
  const swatches = PALETTES.map(
    (p) => `<button class="swatch${s.palette === p.id ? " sel" : ""}" data-set="palette" data-value="${p.id}">
      ${s.palette === p.id ? `<span class="chk">${icon("check", 15)}</span>` : ""}
      <span class="dots">${p.colors.map((c) => `<i style="background:${c}"></i>`).join("")}</span>
      <span class="nm">${p.name}</span>
    </button>`
  ).join("");

  const fontcards = FONTS.map(
    (f) => `<button class="fontcard${s.font === f.id ? " sel" : ""}" data-set="font" data-value="${f.id}" data-font="${f.id}">
      <span class="radio">${s.font === f.id ? icon("check", 14) : ""}</span>
      <div class="fprev">Écho</div>
      <div class="fname">${f.name}</div>
      <div class="fdesc">${f.desc}</div>
    </button>`
  ).join("");

  const thumbs = MODES.map((m) => {
    const inner =
      m.id === "normal"
        ? `<div class="th-side"></div><div class="th-col"><div class="th-line long"></div><div class="th-line"></div><div class="th-line"></div><div class="th-bar"></div></div>`
        : m.id === "compact"
        ? `<div class="th-col center"><div class="th-sq"></div><div class="th-line mid"></div><div class="th-line short"></div><div class="th-thin"></div></div>`
        : `<div class="th-sq"></div><div class="th-col"><div class="th-line long"></div><div class="th-line mid"></div><div class="th-line mid"></div><div class="th-line short"></div></div>`;
    return `<button class="mthumb${s.mode === m.id ? " sel" : ""}" data-set="mode" data-value="${m.id}">
      <div class="th">${inner}</div><div class="mname">${m.name}</div></button>`;
  }).join("");

  return `
  <section class="settings-section" id="set-apparence">
    <h4>Apparence</h4>
    <div class="card">
      ${rowSeg("Couleurs de l'application", "Suivre le système ou forcer clair / sombre", "theme", [
        { value: "system", label: "Système" },
        { value: "light", label: "Clair" },
        { value: "dark", label: "Sombre" },
      ], s.theme)}
    </div>
    <h4 style="margin-top:18px">Palette</h4>
    <div class="swatches">${swatches}</div>
    <h4 style="margin-top:18px">Police</h4>
    <div class="fontcards">${fontcards}</div>
    <h4 style="margin-top:18px">Disposition</h4>
    <div class="mthumbs">${thumbs}</div>
    <h4 style="margin-top:18px">Détails de l'affichage</h4>
    <div class="card">
      ${rowSwitch("Animations", "Transitions douces à l'ouverture des vues", "animations", s.animations)}
      ${rowSwitch("Afficher les pochettes", "Sinon, une note remplace la pochette", "show_covers", s.show_covers)}
      <div class="set-row">
        <div class="left"><div class="lab">Coins arrondis</div><div class="desc">Forme des cartes et des pochettes</div></div>
        <div class="set-ctl"><span class="slider"><input type="range" min="0" max="20" value="${s.radius}" data-slider="radius" aria-label="Coins arrondis" /><span class="hint" id="radius-value">${s.radius} px</span></span></div>
      </div>
      ${rowSeg("Taille du texte", "S'applique à toute l'application", "text_size", [
        { value: "petit", label: "Petit" },
        { value: "normal", label: "Normal" },
        { value: "grand", label: "Grand" },
      ], s.text_size)}
    </div>
  </section>`;
}

function sectionLecture() {
  const s = state.settings;
  return `
  <section class="settings-section" id="set-lecture">
    <h4>Lecture</h4>
    <div class="card">
      ${rowSwitch("Reprendre au dernier morceau", "À l'ouverture, la lecture repart où tu l'as laissée", "resume_last", s.resume_last)}
      ${rowSwitch("Mémoriser la position", "Même au milieu d'un titre", "remember_position", s.remember_position)}
      ${rowSwitch("Aléatoire au démarrage", "Commence en mode mélangé", "shuffle_default", s.shuffle_default)}
      ${rowSwitch("Passer au suivant en cas de problème", "Si un fichier est illisible, on continue", "next_on_error", s.next_on_error)}
      ${rowSeg("Répétition par défaut", "Ce que fait Écho à la fin d'un titre", "repeat_default", [
        { value: "off", label: "Aucune" },
        { value: "all", label: "La file" },
        { value: "one", label: "Un titre" },
      ], s.repeat_default)}
      <div class="set-row">
        <div class="left"><div class="lab">Saut avant / arrière</div><div class="desc">Durée des boutons et des flèches du clavier</div></div>
        <div class="set-ctl">${[5, 10, 30]
          .map((v) => `<button class="chip${s.skip_seconds === v ? " on" : ""}" data-set="skip_seconds" data-value="${v}">${v} s</button>`)
          .join("")}</div>
      </div>
      ${rowSeg("Fondu enchaîné", "Le titre suivant démarre en douceur pendant que l'autre s'éteint", "crossfade", [
        { value: 0, label: "Aucun" },
        { value: 3, label: "3 s" },
        { value: 6, label: "6 s" },
        { value: 10, label: "10 s" },
      ], s.crossfade)}
    </div>
    <h4 style="margin-top:18px">Son</h4>
    <div class="card">
      ${rowSwitch("Volume égal", "Rattrape les différences de niveau entre les morceaux, sans jamais saturer", "loudness", s.loudness)}
      <div class="set-row eq-row">
        <div class="left"><div class="lab">Égaliseur</div><div class="desc">Couleur du son, appliquée à tous les morceaux</div></div>
        <div class="set-ctl eq-ctl">${EQ_CHOICES.map(
          (o) => `<button class="chip${s.eq === o.value ? " on" : ""}" data-set="eq" data-value="${o.value}">${o.label}</button>`
        ).join("")}</div>
      </div>
    </div>
    <h4 style="margin-top:18px">Paroles</h4>
    <div class="card">
      ${rowSwitch("Chercher les paroles en ligne", "Sur LRCLIB, une base libre, quand le fichier n'en a pas. Seuls l'artiste, le titre et la durée sont envoyés.", "lyrics_online", s.lyrics_online)}
      ${rowSwitch("Chercher les pochettes en ligne", "Pour les morceaux sans pochette ou avec une image de clip, sur MusicBrainz (base libre). Seuls l'artiste, le titre et l'album sont envoyés.", "covers_online", false, null, true)}
    </div>
  </section>`;
}

function sectionBiblio() {
  const s = state.settings;
  const folders = state.folders.length
    ? state.folders
        .map(
          (f) => `<div class="path-row">
        <span class="path" title="${escapeHtml(f)}">${escapeHtml(f)}</span>
        <button class="icon-btn" data-remove-folder="${escapeHtml(f)}" title="Retirer">${icon("close", 15)}</button>
      </div>`
        )
        .join("")
    : `<div class="path-row"><span class="hint">Aucun dossier pour l'instant. Ajoute celui où tu ranges ta musique.</span></div>`;

  const formats = state.formats
    .map(
      (f) => `<button class="chip${s.formats.includes(f) ? " on" : ""}" data-set="formats" data-value="${f}">${FORMAT_NAMES[f] || f}</button>`
    )
    .join("");

  const mo = (state.cacheBytes / (1024 * 1024)).toFixed(1).replace(".", ",");
  return `
  <section class="settings-section" id="set-bibliotheque">
    <h4>Bibliothèque</h4>
    <div class="card">
      ${folders}
      <div class="path-row">
        <button class="primary-btn" data-action="add-folder">${icon("plus", 15)} Ajouter un dossier</button>
        <span class="hint">Les fichiers sont lus sur place, rien n'est copié.</span>
      </div>
    </div>
    <h4 style="margin-top:18px">Formats acceptés</h4>
    <div class="card">
      <div class="set-row">
        <div class="left"><div class="lab">Extensions lues par la bibliothèque</div><div class="desc">Décoche ce que tu veux éviter de voir</div></div>
        <div class="set-ctl formats-ctl">${formats}</div>
      </div>
      ${rowSwitch("Actualiser au démarrage", "Vérifie la bibliothèque à chaque ouverture", "refresh_on_start", s.refresh_on_start)}
      ${rowSwitch("Ajouter les nouveaux morceaux automatiquement", "Un fichier ajouté, déplacé ou supprimé dans tes dossiers est pris en compte tout seul", "watch", s.watch)}
    </div>
    <h4 style="margin-top:18px">Pochettes en cache</h4>
    <div class="card">
      ${rowButton("Cache (pochettes, son des clips)", `${mo} Mo sur le disque`, `<button class="ghost-btn" data-action="clear-cache">Vider</button>`)}
    </div>
    <div class="card" style="margin-top:10px">
      ${rowButton("Relire les dossiers", "Utile si tu viens d'ajouter des fichiers", `<button class="ghost-btn" data-action="rescan">${icon("refresh", 15)} Actualiser</button>`)}
    </div>
  </section>`;
}

function sectionSysteme() {
  const s = state.settings;
  return `
  <section class="settings-section" id="set-systeme">
    <h4>Système</h4>
    <div class="card">
      ${rowSwitch("Touches média du clavier", "Les boutons lecture / suivant de ton clavier", "media_keys", s.media_keys)}
      ${rowSwitch("Démarrer avec la session", "Écho s'ouvre à l'allumage de l'ordinateur", "autostart", s.autostart)}
      ${rowSwitch("Icône dans la barre du système", "Lecture, suivant, précédent et retour à la fenêtre, depuis la barre", "tray", s.tray)}
      ${rowSwitch("Notification à chaque morceau", "Un petit bandeau quand le titre change et qu'Écho est en arrière-plan", "notify", s.notify)}
      ${rowSwitch("Statut Discord", "Affiche « Écoute… » avec le titre et l'artiste sur ton profil Discord", "discord", s.discord)}
      ${rowSeg("Quand je ferme la fenêtre", s.tray ? "Ce que fait le bouton de fermeture" : "« Réduire » range Écho dans la barre du système (à activer ci-dessus)", "close", [
        { value: "quit", label: "Quitter" },
        { value: "ask", label: "Demander" },
        ...(s.tray ? [{ value: "tray", label: "Réduire" }] : []),
      ], s.close_to_tray && s.tray ? "tray" : s.ask_before_quit ? "ask" : "quit")}
    </div>
    <h4 style="margin-top:18px">Mises à jour</h4>
    <div class="card">
      ${rowSeg("Mises à jour", "Versions signées : un fichier qui ne vient pas d'Écho est refusé", "updates", [
        { value: "auto", label: "Automatiques" },
        { value: "notify", label: "Me prévenir" },
        { value: "off", label: "Jamais" },
      ], s.updates)}
      ${updateRow()}
    </div>
  </section>`;
}

function sectionDonnees() {
  return `
  <section class="settings-section" id="set-donnees">
    <h4>Données</h4>
    <div class="card">
      <div class="path-row">
        <span class="path">~/.config/com.echo.player</span>
        <button class="ghost-btn" data-action="open-config">${icon("folder", 15)} Ouvrir le dossier</button>
      </div>
      ${rowButton("Sauvegarde des réglages", "Un fichier que tu peux garder de côté", `
        <button class="ghost-btn" data-action="export">${icon("share", 15)} Exporter…</button>
        <button class="ghost-btn" data-action="import">${icon("download", 15)} Importer…</button>`)}
      ${rowButton("Réinitialiser les réglages", "Remet tout à l'état d'origine", `<button class="ghost-btn" data-action="reset" style="color:var(--danger);border-color:color-mix(in srgb, var(--danger) 35%, transparent)">Réinitialiser</button>`)}
    </div>
  </section>`;
}

function sectionAbout() {
  return `
  <section class="settings-section" id="set-apropos">
    <h4>À propos</h4>
    <div class="card">
      <div class="about">
        <div class="about-mark">${icon("waveform", 24)}</div>
        <div>
          <div class="about-name">Écho <span class="badge">${escapeHtml(state.version || "")}</span></div>
          <div class="about-desc">Lecteur de musique, pour tes fichiers, sur ton ordinateur.</div>
          <div class="about-facts">
            <span>${icon("dashboard", 13)} Léger sur la mémoire</span>
            <span>${icon("check", 13)} Gratuit et libre</span>
            <span>${icon("sparkles", 13)} Icônes Hugeicons</span>
            <span>${icon("magic", 13)} Police Space Grotesk</span>
          </div>
          <div class="about-promise">Aucun compte, aucune publicité, aucune connexion : tout reste chez toi.</div>
        </div>
      </div>
    </div>
  </section>`;
}

function buildSettingsPanel() {
  const panel = document.createElement("div");
  panel.className = "settings-panel";
  panel.id = "settings-panel";
  panel.dataset.home = "settings-host-normal";
  panel.innerHTML = `
    <div class="settings-nav">
      ${[
        ["Apparence", "palette"],
        ["Lecture", "headphones"],
        ["Bibliothèque", "library"],
        ["Système", "settings"],
        ["Données", "history"],
        ["À propos", "dashboard"],
      ]
        .map(([name, ic], i) => `<button data-goto="${i}" class="${i === 0 ? "on" : ""}">${icon(ic, 15)}<span>${name}</span></button>`)
        .join("")}
    </div>
    <div class="settings-body" id="settings-body">
      <h3>Réglages</h3>
      <p class="hint">Tout se règle ici, et tout reste sur ton ordinateur.</p>
      ${sectionApparence()}
      ${sectionLecture()}
      ${sectionBiblio()}
      ${sectionSysteme()}
      ${sectionDonnees()}
      ${sectionAbout()}
    </div>`;
  bindSettings(panel);
  return panel;
}

/// Un sélecteur qui retrouve le même contrôle après reconstruction du panneau.
function controlSelector(node) {
  if (node.dataset.set) return `[data-set="${node.dataset.set}"][data-value="${node.dataset.value}"]`;
  if (node.dataset.toggle) return `[data-toggle="${node.dataset.toggle}"]`;
  if (node.dataset.slider) return `[data-slider="${node.dataset.slider}"]`;
  if (node.dataset.action) return `[data-action="${node.dataset.action}"]`;
  if (node.dataset.goto) return `[data-goto="${node.dataset.goto}"]`;
  return null;
}

/// Met `a` à l'image de `b` en ne touchant qu'à ce qui diffère : les éléments restent
/// les mêmes (interrupteurs qui glissent, survol et focus gardés, rien ne clignote).
function morph(a, b) {
  if (a.nodeType !== b.nodeType || a.nodeName !== b.nodeName) {
    a.replaceWith(b);
    return;
  }
  if (a.nodeType === Node.TEXT_NODE || a.nodeType === Node.COMMENT_NODE) {
    if (a.nodeValue !== b.nodeValue) a.nodeValue = b.nodeValue;
    return;
  }
  if (a.nodeType !== Node.ELEMENT_NODE) return;
  for (const { name } of [...a.attributes]) if (!b.hasAttribute(name)) a.removeAttribute(name);
  for (const { name, value } of [...b.attributes]) if (a.getAttribute(name) !== value) a.setAttribute(name, value);
  if ((a.tagName === "INPUT" || a.tagName === "SELECT") && a.value !== b.value && document.activeElement !== a) a.value = b.value;
  const ac = [...a.childNodes];
  const bc = [...b.childNodes];
  const n = Math.min(ac.length, bc.length);
  for (let i = 0; i < n; i++) morph(ac[i], bc[i]);
  for (let i = n; i < ac.length; i++) ac[i].remove();
  for (let i = n; i < bc.length; i++) a.appendChild(bc[i]);
}

function refreshSettingsPanel() {
  const old = el("settings-panel");
  if (!old) return;
  // Mise à jour sur place : le panneau garde ses éléments (et ses écouteurs, posés sur
  // le panneau lui-même), son défilement et le focus.
  const oldBody = old.querySelector(".settings-body");
  const fresh = buildSettingsPanel();
  const freshBody = fresh.querySelector(".settings-body");
  if (oldBody && freshBody) {
    morph(oldBody, freshBody);
    syncSettingsNav();
    return;
  }
  // Le panneau est reconstruit : on garde la position de défilement et le focus clavier.
  const active = document.activeElement;
  const focusKey = active && old.contains(active) ? controlSelector(active) : null;
  const oldNav = document.querySelector(".settings-nav");
  const body = old.querySelector(".settings-body");
  const scroll = body ? body.scrollTop : 0;
  // Le nouveau panneau prend la place de l'ancien, y compris dans une fenêtre superposée :
  // sinon, à la fermeture, il ne retrouverait pas son emplacement et disparaîtrait.
  if (HOMES.has(old)) {
    HOMES.set(fresh, HOMES.get(old));
    HOMES.delete(old);
  }
  old.replaceWith(fresh);
  // La rubrique gardée à l'écran est celle d'origine (avec ses écouteurs) : on la reprend.
  const freshNav = fresh.querySelector(".settings-nav");
  if (oldNav && freshNav) freshNav.replaceWith(oldNav);
  syncSettingsNav();
  const newBody = fresh.querySelector(".settings-body");
  if (newBody) newBody.scrollTop = scroll;
  if (focusKey) {
    const again = fresh.querySelector(focusKey);
    if (again) again.focus({ preventScroll: true });
  }
}

/// En disposition normale, dans la rubrique Réglages, les rubriques prennent la place
/// de la barre latérale (comme dans les maquettes). Ailleurs, elles restent dans le panneau.
function syncSettingsNav() {
  const navEl = document.querySelector(".settings-nav");
  const panel = el("settings-panel");
  const sidebar = document.querySelector(".sidebar");
  const mainNav = el("nav");
  if (!navEl || !panel || !sidebar || !mainNav) return;
  const inSidebar = state.view === "settings" && document.body.dataset.mode === "normal";
  if (inSidebar) {
    if (navEl.parentElement !== sidebar) mainNav.after(navEl);
  } else if (navEl.parentElement !== panel) {
    panel.prepend(navEl);
  }
  // Les rubriques s'ajoutent sous les vues principales : on garde le retour à la bibliothèque.
  mainNav.hidden = false;
  panel.classList.toggle("nav-moved", inSidebar);
}

const SET_SECTIONS = ["set-apparence", "set-lecture", "set-bibliotheque", "set-systeme", "set-donnees", "set-apropos"];

function bindSettings(panel) {
  panel.querySelector(".settings-nav").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-goto]");
    if (!btn) return;
    const target = el(SET_SECTIONS[Number(btn.dataset.goto)]);
    if (target) target.scrollIntoView({ block: "start", behavior: state.settings.animations ? "smooth" : "auto" });
    document.querySelectorAll(".settings-nav button").forEach((b) => b.classList.toggle("on", b === btn));
  });

  const body = panel.querySelector(".settings-body");
  body.addEventListener("scroll", () => {
    const top = body.scrollTop + 40;
    let index = 0;
    SET_SECTIONS.forEach((id, i) => {
      const node = el(id);
      if (node && node.offsetTop - 120 <= top) index = i;
    });
    document.querySelectorAll(".settings-nav button").forEach((b, i) => b.classList.toggle("on", i === index));
  });

  panel.addEventListener("change", (e) => {
    const slider = e.target.closest("[data-slider]");
    if (!slider) return;
    const field = slider.dataset.slider;
    const value = Number(slider.value);
    const label = slider.parentElement.querySelector("#radius-value");
    if (label) label.textContent = `${value} px`;
    saveSettings({ [field]: value });
  });

  panel.addEventListener("input", (e) => {
    const slider = e.target.closest("[data-slider]");
    if (!slider) return;
    const value = Number(slider.value);
    const label = slider.parentElement.querySelector("#radius-value");
    if (label) label.textContent = `${value} px`;
    state.settings[slider.dataset.slider] = value;
    applySettings();
  });

  panel.addEventListener("click", async (e) => {
    const toggle = e.target.closest("[data-toggle]");
    if (toggle) {
      const field = toggle.dataset.toggle;
      const on = !state.settings[field];
      // Sans icône dans la barre, la fenêtre ne peut plus s'y ranger.
      saveSettings(field === "tray" && !on ? { tray: false, close_to_tray: false } : { [field]: on });
      if (field === "discord" && on && !state.discordReady) {
        toast("Statut Discord : il manque l'identifiant de l'application Discord (voir le README).");
      }
      return;
    }

    const choice = e.target.closest("[data-set]");
    if (choice) {
      const field = choice.dataset.set;
      const value = choice.dataset.value;
      if (field === "formats") {
        const list = new Set(state.settings.formats);
        if (list.has(value)) list.delete(value);
        else list.add(value);
        if (list.size === 0) {
          toast("Garde au moins un format, sinon la bibliothèque serait vide.");
          return;
        }
        saveSettings({ formats: [...list] });
      } else if (field === "skip_seconds" || field === "crossfade") {
        saveSettings({ [field]: Number(value) });
      } else if (field === "close") {
        saveSettings({ ask_before_quit: value === "ask", close_to_tray: value === "tray" });
      } else {
        saveSettings({ [field]: value });
      }
      return;
    }

    const remove = e.target.closest("[data-remove-folder]");
    if (remove) {
      try {
        const snap = await invoke("remove_folder", { path: remove.dataset.removeFolder });
        applySnapshot(snap);
        refreshSettingsPanel();
      } catch (err) {
        console.error(err);
      }
      return;
    }

    const action = e.target.closest("[data-action]");
    if (!action) return;
    switch (action.dataset.action) {
      case "update-check":
        updateAction("check");
        break;
      case "update-install":
        updateAction("install");
        break;
      case "update-restart":
        updateAction("restart");
        break;
      case "update-page":
        updateAction("page");
        break;
      case "add-folder":
        addFolder();
        break;
      case "rescan":
        await rescan();
        refreshSettingsPanel();
        break;
      case "clear-cache": {
        toast("Pochettes vidées, relecture des fichiers…");
        const cleared = await invoke("clear_cover_cache");
        coverUrls.clear();
        thumbUrls.clear();
        frameTried.clear();
        audioPaths.clear();
        state.cacheBytes = cleared.cache_bytes;
        // Mêmes morceaux, mais toutes les pochettes sont à reposer.
        snapshotSig = "";
        libraryVersion += 1;
        applySnapshot(cleared.snapshot);
        refreshSettingsPanel();
        toast("Les pochettes ont été reconstruites.");
        break;
      }
      case "open-config":
        await invoke("open_config_dir").catch(() => toast("Impossible d'ouvrir le dossier."));
        break;
      case "export": {
        const path = await saveDialog({
          title: "Où garder la sauvegarde ?",
          defaultPath: "reglages-echo.json",
          filters: [{ name: "Réglages Écho", extensions: ["json"] }],
        });
        if (!path) return;
        try {
          await invoke("export_settings", { path });
          toast("Réglages exportés.");
        } catch (err) {
          toast(String(err));
        }
        break;
      }
      case "import": {
        const picked = await openDialog({
          title: "Choisir une sauvegarde de réglages",
          filters: [{ name: "Réglages Écho", extensions: ["json"] }],
        });
        if (!picked) return;
        const path = typeof picked === "string" ? picked : picked.path;
        try {
          const boot = await invoke("import_settings", { path });
          await applyBootstrap(boot);
          toast("Réglages importés.");
        } catch (err) {
          toast(String(err));
        }
        break;
      }
      case "reset": {
        const ok = await ask("Tout remettre à zéro ?", "Tes dossiers et tes réglages reviendront à l'état d'origine.", [
          { label: "Annuler", value: false },
          { label: "Réinitialiser", value: true, primary: true },
        ]);
        if (!ok) return;
        const boot = await invoke("reset_settings");
        await applyBootstrap(boot);
        toast("Réglages remis à zéro.");
        break;
      }
    }
  });
}

/* ============================================================
   Liste de la bibliothèque (virtuelle)
   ============================================================ */

/// Liste filtrée et triée, gardée tant que ni la bibliothèque, ni la recherche,
/// ni le tri ne changent : inutile de tout retrier à chaque lecture.
let libraryVersion = 0;
let visibleMemo = { version: -1, search: null, sort: null, list: [] };

function visibleTracks() {
  if (
    visibleMemo.version !== libraryVersion ||
    visibleMemo.search !== state.search ||
    visibleMemo.sort !== state.sort
  ) {
    visibleMemo = {
      version: libraryVersion,
      search: state.search,
      sort: state.sort,
      list: computeVisibleTracks(),
    };
  }
  return visibleMemo.list;
}

function computeVisibleTracks() {
  const q = normalize(state.search);
  let list = audioTracks();
  if (q) list = list.filter((t) => searchKey(t).includes(q));
  const key = state.sort;
  const dir = key === "duration" ? -1 : 1;
  return [...list].sort((a, b) => {
    if (key === "duration") return (a.duration_ms - b.duration_ms) * dir;
    return COLLATOR.compare(String(a[key] || ""), String(b[key] || ""));
  });
}

function rowHtml(track, index, current, playing) {
  return `
    <div class="row-index">${current && playing ? `<span class="eq">${ICONS.waveform}</span>` : index + 1}</div>
    <div class="row-main">
      <div class="thumb"></div>
      <div class="row-text">
        <div class="row-title">${escapeHtml(track.title)}</div>
        <div class="row-artist">${escapeHtml(track.artist || "—")}</div>
      </div>
    </div>
    <div class="row-album">${escapeHtml(track.album || "—")}</div>
    <div class="row-time">${track.has_audio === false ? '<span class="badge mute-badge" title="Vidéo sans piste son">muet</span>' : track.duration_ms ? formatTime(track.duration_ms) : "—"}</div>`;
}

/// Liste virtuelle : seules les lignes visibles existent, et une ligne déjà dessinée
/// reste en place au défilement (sinon ses pochettes clignotent à chaque image).
function virtualList(container, count, rowHeight, makeRow) {
  let spacer = container.querySelector(":scope > .list-spacer");
  if (!spacer) {
    spacer = document.createElement("div");
    spacer.className = "list-spacer";
    container.replaceChildren(spacer);
  }
  spacer.style.height = `${count * rowHeight}px`;
  spacer.replaceChildren();
  const rows = new Map();
  const draw = () => {
    // Liste cachée (panneau fermé, onglet inactif) : rien à mesurer, on garde les lignes.
    if (!container.clientHeight) return;
    const top = container.scrollTop;
    // Une hauteur d'écran de marge de chaque côté : au défilement rapide, les lignes
    // sont déjà là (pas de bandes vides).
    const pad = Math.max(6, Math.ceil(container.clientHeight / rowHeight));
    const first = Math.max(0, Math.floor(top / rowHeight) - pad);
    const last = Math.min(count, Math.ceil((top + container.clientHeight) / rowHeight) + pad);
    for (const [i, node] of rows) {
      if (i < first || i >= last) {
        node.remove();
        rows.delete(i);
      }
    }
    for (let i = first; i < last; i++) {
      if (rows.has(i)) continue;
      const node = makeRow(i);
      if (!node) continue;
      node.style.top = `${i * rowHeight}px`;
      rows.set(i, node);
      spacer.appendChild(node);
    }
  };
  container.onscroll = onFrame(draw);
  container.vlDraw = draw;
  // Le panneau qui s'ouvre, la fenêtre qu'on agrandit : de nouvelles lignes deviennent visibles.
  if (!container.vlObserved && window.ResizeObserver) {
    container.vlObserved = true;
    new ResizeObserver(() => container.vlDraw && container.vlDraw()).observe(container);
  }
  draw();
  return rows;
}

/// Lignes de la bibliothèque à l'écran, et la liste qu'elles montrent.
let libRows = new Map();
let libShown = null;

function renderLibrary() {
  const listEl = el("list");
  if (!listEl) return;
  const all = visibleTracks();
  state.listOrder = all.map((t) => t.id);
  // Liste vide : on dit pourquoi (recherche sans résultat, ou rien encore).
  const empty = all.length
    ? ""
    : state.search
      ? `Aucun morceau pour « ${state.search} ».`
      : state.tracks.length
        ? "Aucun morceau à afficher."
        : "Aucun morceau pour l'instant.";
  if ((listEl.dataset.empty || "") !== empty) {
    if (empty) listEl.dataset.empty = empty;
    else delete listEl.dataset.empty;
  }
  // Même liste qu'avant (lecture, pause, changement de vue) : on ne met à jour que l'état.
  if (all === libShown && listEl.querySelector(":scope > .list-spacer")) {
    if (listEl.vlDraw) listEl.vlDraw();
    markLibrary();
    return;
  }
  libShown = all;
  libRows = virtualList(listEl, all.length, ROW_HEIGHT, (i) => {
    const track = all[i];
    const row = document.createElement("div");
    row.dataset.index = String(i);
    row.dataset.trackId = track.id;
    row.innerHTML = rowHtml(track, i, false, false);
    fillCover(row.querySelector(".thumb"), track);
    row.addEventListener("click", () => playFromList(i));
    paintRowState(row, i);
    return row;
  });
}

/// État « en cours » / « en lecture » d'une ligne, sans la redessiner.
function paintRowState(row, i) {
  const current = row.dataset.trackId === currentId();
  const playing = current && isPlaying();
  const wanted = playing ? "playing" : current ? "current" : "";
  if (row.dataset.state === wanted) return;
  row.dataset.state = wanted;
  row.className = "row" + (current ? " is-current" : "") + (playing ? " is-playing" : "");
  row.querySelector(".row-index").innerHTML = playing ? `<span class="eq">${ICONS.waveform}</span>` : String(i + 1);
}

function markLibrary() {
  for (const [i, row] of libRows) paintRowState(row, i);
}


const QUEUE_ROW = 40;

let queueShown = "";
let queueRows = new Map();

function renderQueue() {
  syncNativeQueue();
  const queue = el("queue");
  if (!queue) return;
  const ids = state.queue;
  const count = el("queue-count");
  if (count) setText(count, ids.length ? countLabel(ids.length) : "");

  if (ids.length === 0) {
    queueShown = "";
    queueRows = new Map();
    const p = document.createElement("div");
    p.className = "hint";
    p.textContent = "La file est vide. Choisis un morceau dans la bibliothèque.";
    queue.replaceChildren(p);
    queue.onscroll = null;
    return;
  }

  // Même file qu'avant : on ne déplace que la marque du morceau en cours.
  const key = `${libraryVersion}\n${ids.join("\n")}`;
  if (key === queueShown && queue.querySelector(":scope > .list-spacer")) {
    if (queue.vlDraw) queue.vlDraw();
    for (const [, item] of queueRows) item.classList.toggle("is-current", item.dataset.trackId === currentId());
    return;
  }
  queueShown = key;

  // Une seule table id → morceau, au lieu d'une recherche par ligne.
  const byId = new Map(state.tracks.map((t) => [t.id, t]));
  queueRows = virtualList(queue, ids.length, QUEUE_ROW, (i) => {
    const id = ids[i];
    const track = byId.get(id);
    if (!track) return null;
    const item = document.createElement("button");
    item.className = "queue-item" + (id === currentId() ? " is-current" : "");
    item.dataset.trackId = id;
    item.innerHTML = `<span class="queue-num">${i + 1}</span>
        <span class="queue-name">${escapeHtml(track.title)}${track.artist ? " — " + escapeHtml(track.artist) : ""}</span>
        <span class="queue-time">${track.duration_ms ? formatTime(track.duration_ms) : "—"}</span>`;
    item.addEventListener("click", () => {
      state.queueIndex = i;
      playCurrent();
    });
    return item;
  });
}

/* ============================================================
   Mode compacte : carrousel
   ============================================================ */

/// Ce que montre le carrousel : il n'est redessiné que si ça change.
let carouselShown = "";

function renderCarousel() {
  const box = el("carousel");
  if (!box || state.settings.mode !== "compact") return;
  const playing = isPlaying();
  const now = currentId();
  const changed = now !== lastCarouselId;
  lastCarouselId = now;
  // Mêmes pochettes autour du même morceau (lecture / pause) : on ne change que l'état.
  const n = state.queue.length || 1;
  const key = `${libraryVersion}|${state.settings.show_covers}|${state.queueIndex}|${state.queue.length}|${[-2, -1, 0, 1, 2]
    .map((o) => state.queue[(((state.queueIndex + o) % n) + n) % n])
    .join("|")}`;
  if (key === carouselShown && box.firstChild) {
    const nowNode = box.querySelector(".car-cover.now");
    if (nowNode) {
      nowNode.classList.toggle("playing", playing);
      let eq = nowNode.querySelector(".playing-eq");
      if (playing && !eq) {
        eq = document.createElement("span");
        eq.className = "playing-eq";
        eq.innerHTML = ICONS.waveform;
        nowNode.appendChild(eq);
      } else if (!playing && eq) {
        eq.remove();
      }
    }
    return;
  }
  carouselShown = key;
  box.replaceChildren();
  if (!state.queue.length) {
    box.innerHTML = `<div class="car-cover now" style="width:190px;height:190px">${icon("note3", 40)}</div>`;
    return;
  }
  // Nombre impair de pochettes pour que celle du morceau en cours reste centrée.
  const total = state.queue.length;
  const slots = total >= 5 ? 5 : total >= 3 ? (total % 2 === 0 ? 3 : total) : 1;
  const around = [];
  for (let i = -(slots - 1) / 2; i <= (slots - 1) / 2; i++) around.push(i);

  for (const offset of around) {
    const index = ((state.queueIndex + offset) % total + total) % total;
    const id = state.queue[index];
    const track = trackById(id);
    const node = document.createElement("div");
    node.className = "car-cover " + (offset === 0 ? "now" : "side");
    const size = offset === 0 ? 190 : Math.abs(offset) === 1 ? 138 : 104;
    node.style.width = `${size}px`;
    node.style.height = `${size}px`;
    // Taille connue d'avance : l'élément n'est pas encore dans la page, on ne peut pas la mesurer.
    node.dataset.coverSize = size * (window.devicePixelRatio || 1) <= 128 ? "s" : "l";
    if (!track) {
      node.style.opacity = ".12";
      box.appendChild(node);
      continue;
    }
    const isNow = offset === 0;
    if (isNow && playing) node.classList.add("playing");
    if (isNow && changed) node.classList.add("swap");
    fillCover(node, track);
    if (isNow && playing) {
      const eq = document.createElement("span");
      eq.className = "playing-eq";
      eq.innerHTML = ICONS.waveform;
      node.appendChild(eq);
    }
    node.addEventListener("click", () => {
      if (isNow) togglePlay();
      else {
        state.queueIndex = index;
        playCurrent();
      }
    });
    box.appendChild(node);
  }
}

/* ============================================================
   Mode minimaliste : album
   ============================================================ */

/// Albums de la bibliothèque, regroupés une fois par version de la bibliothèque.
let albumMemo = { version: -1, list: [] };

function albumList() {
  if (albumMemo.version !== libraryVersion) albumMemo = { version: libraryVersion, list: computeAlbumList() };
  return albumMemo.list;
}

function computeAlbumList() {
  const map = new Map();
  for (const track of audioTracks()) {
    const name = track.album || "Sans album";
    const key = `${normalize(name)}::${normalize(track.artist || "")}`;
    if (!map.has(key)) map.set(key, { key, name, artist: track.artist || "", tracks: [] });
    map.get(key).tracks.push(track);
  }
  return [...map.values()].sort((a, b) => COLLATOR.compare(a.name, b.name));
}

function currentAlbum() {
  const albums = albumList();
  if (!albums.length) return null;
  if (state.minimalAlbum) {
    const found = albums.find((a) => a.key === state.minimalAlbum);
    if (found) return found;
  }
  const track = currentTrack();
  if (track) {
    const found = albums.find((a) => a.tracks.some((t) => t.id === track.id));
    if (found) return found;
  }
  return albums[0];
}

/// Morceau en cours et onde de lecture dans la liste du mode minimaliste, sans la refaire.
function markMinimal(right) {
  const playing = isPlaying();
  right.querySelectorAll("[data-min-id]").forEach((node, i) => {
    const current = node.dataset.minId === currentId();
    node.classList.toggle("is-current", current);
    const first = node.firstElementChild;
    const wantWave = current && playing;
    if (first && first.classList.contains("playing") !== wantWave) {
      first.outerHTML = wantWave ? `<span class="playing">${ICONS.waveform}</span>` : `<span class="trk-n">${i + 1}</span>`;
    }
  });
}

function renderMinimal() {
  const right = el("min-right");
  if (!right || state.settings.mode !== "minimal") return;

  if (state.minSearch.trim()) {
    // Mêmes résultats qu'au dernier rendu : seul l'état change (la liste garde sa place).
    const shownKey = `q:${state.minSearch}|${libraryVersion}`;
    if (right.dataset.shown === shownKey && right.querySelector(".album-tracks")) return markMinimal(right);
    right.dataset.shown = shownKey;
    const q = normalize(state.minSearch);
    const matches = audioTracks().filter((t) => searchKey(t).includes(q));
    const results = matches.slice(0, 200);
    const capped = matches.length > results.length ? " · 200 premiers affichés" : "";
    right.innerHTML = `
      <div class="album-title">Résultats</div>
      <div class="album-meta">${countLabel(matches.length)} pour « ${escapeHtml(state.minSearch)} »${capped}</div>
      <div class="album-tracks">${results
        .map((t, i) => minimalTrackRow(t, i, true))
        .join("")}</div>`;
    right.querySelectorAll("[data-min-id]").forEach((node) => {
      node.addEventListener("click", (e) => {
        if (openMenuFromDots(e)) return;
        playFromResults(node.dataset.minId, results.map((t) => t.id));
      });
    });
    return;
  }

  const album = currentAlbum();
  if (!album) {
    right.dataset.shown = "";
    right.innerHTML = `<div class="album-title">Aucun album</div>
      <div class="album-meta">Choisis un dossier dans les réglages, ou ajoute un morceau à ta bibliothèque.</div>`;
    return;
  }

  // Même album qu'au dernier rendu (lecture, pause, morceau suivant) : seul l'état change.
  const key = `${album.key}|${libraryVersion}`;
  if (right.dataset.shown === key && right.querySelector(".album-tracks")) return markMinimal(right);
  right.dataset.shown = key;

  const total = album.tracks.reduce((sum, t) => sum + (t.duration_ms || 0), 0);
  right.innerHTML = `
    <div class="album-title">${escapeHtml(album.name)}</div>
    <div class="album-meta">${album.artist ? escapeHtml(album.artist) + " · " : ""}${countLabel(album.tracks.length)} · ${formatTime(total)}</div>
    <div class="album-tracks">${album.tracks.map((t, i) => minimalTrackRow(t, i, false)).join("")}</div>`;

  right.querySelectorAll("[data-min-id]").forEach((node) => {
    node.addEventListener("click", (e) => {
      if (openMenuFromDots(e)) return;
      state.queue = album.tracks.map((t) => t.id);
      state.queueIndex = album.tracks.findIndex((t) => t.id === node.dataset.minId);
      playCurrent();
    });
  });

  const cover = el("min-cover");
  fillCover(cover, album.tracks[0], false);
  if (cover.dataset.album !== album.key) {
    // Changement d'album : la pochette et la fiche s'animent ensemble.
    playAnim(cover, "swap");
    playAnim(right, "swap-text");
  }
  cover.dataset.album = album.key;

  const albums = albumList();
  const index = albums.findIndex((a) => a.key === album.key);
  const prev = albums[(index - 1 + albums.length) % albums.length];
  const next = albums[(index + 1) % albums.length];
  setPeek(el("peek-left"), prev, album.key);
  setPeek(el("peek-right"), next, album.key);
}

/// Le bouton « … » d'une ligne ouvre le menu du morceau au lieu de le lire.
function openMenuFromDots(e) {
  const dots = e.target.closest("[data-menu]");
  if (!dots) return false;
  e.stopPropagation();
  const track = trackById(dots.dataset.menu);
  const rect = dots.getBoundingClientRect();
  if (track) openTrackMenu(track, rect.left - 180, rect.bottom + 4);
  return true;
}

function minimalTrackRow(track, index, showAlbum) {
  const playing = isPlaying();
  const current = track.id === currentId();
  const left = current && playing ? `<span class="playing">${ICONS.waveform}</span>` : `<span class="trk-n">${index + 1}</span>`;
  return `<button class="trk${current ? " is-current" : ""}" data-min-id="${escapeHtml(track.id)}">
    ${left}
    <span class="trk-t">${escapeHtml(track.title)}${showAlbum && track.album ? ` <span class="hint">${escapeHtml(track.album)}</span>` : ""}</span>
    <span class="trk-d">${track.has_audio === false ? '<span class="badge mute-badge">muet</span>' : track.duration_ms ? formatTime(track.duration_ms) : "—"}</span>
    <span class="trk-m" data-menu="${escapeHtml(track.id)}" role="button" aria-label="Plus d'actions" title="Plus d'actions">${icon("more", 15)}</span>
  </button>`;
}

function setPeek(node, album, currentKey) {
  if (!node) return;
  if (!album || album.key === currentKey) {
    node.hidden = true;
    return;
  }
  node.hidden = false;
  fillCover(node, album.tracks[0], false);
  node.onclick = () => {
    state.minimalAlbum = album.key;
    renderMinimal();
  };
}

/* ============================================================
   Lecture
   ============================================================ */

/// Le lecteur actif. Pendant un fondu enchaîné, l'autre lecteur finit de s'éteindre.
let audio = Sound.element;

/// Vrai quand le cœur lit la musique (Linux) : la page n'est qu'un écran et une
/// télécommande. Elle peut être fermée sans couper le son (barre du système).
let NATIVE = false;

/// Télécommande du lecteur du cœur, avec les mêmes propriétés qu'un lecteur <audio>
/// (lecture, position, durée) : le reste de la page s'en sert sans le savoir.
/// La position avance toute seule entre deux nouvelles du cœur (une par seconde).
const nativeMedia = {
  paused: true,
  duration: NaN,
  pos: 0,
  at: 0,
  /// Morceau que le cœur joue (même s'il a disparu de la bibliothèque entre-temps).
  id: null,
  get currentTime() {
    if (this.paused) return this.pos;
    const t = this.pos + (performance.now() - this.at) / 1000;
    return isFinite(this.duration) ? Math.min(t, this.duration) : t;
  },
  set currentTime(v) {
    this.pos = Math.max(0, v);
    this.at = performance.now();
    invoke("player_seek", { ms: Math.round(this.pos * 1000) }).catch(() => {});
  },
  play() {
    flushNativeQueue();
    this.at = performance.now();
    this.paused = false;
    invoke("player_set_playing", { playing: true }).catch(() => {});
    renderPlayState();
    return Promise.resolve();
  },
  pause() {
    if (!this.paused) this.pos = this.currentTime;
    this.paused = true;
    invoke("player_set_playing", { playing: false }).catch(() => {});
    renderPlayState();
  },
  update(s) {
    const before = this.currentTime;
    const pos = s.position_ms / 1000;
    // La position interpolée a pu prendre quelques centièmes d'avance : on ne la fait
    // pas reculer pour si peu (les paroles s'éteindraient puis se rallumeraient).
    const keep = !s.changed && s.playing && !this.paused && before - pos > 0 && before - pos < 0.25;
    this.paused = !s.playing;
    this.pos = keep ? before : pos;
    this.at = performance.now();
    this.id = s.id || null;
    // Durée pas encore lue par le cœur : celle des tags, sinon la barre disparaîtrait.
    const track = s.id ? trackById(s.id) : null;
    const ms = s.duration_ms > 0 ? s.duration_ms : (track && track.duration_ms) || 0;
    this.duration = ms > 0 ? ms / 1000 : NaN;
  },
};

/// Pendant la reprise d'une page rouverte, la file locale n'est pas encore la bonne :
/// rien n'est envoyé au cœur, qui garde la sienne.
let nativeAdopting = false;
/// Dernier statut reçu : les statuts arrivés en retard (d'un autre fil) sont ignorés.
let lastSeq = -1;
/// Morceau demandé au cœur, pas encore confirmé.
let nativeExpect = null;

/// Passe la lecture au cœur. Renvoie vrai si une lecture y était déjà en cours
/// (page rouverte depuis la barre du système) : la page reprend alors son état.
async function enableNative() {
  nativeAdopting = true;
  try {
    if (!NATIVE) {
      NATIVE = true;
      Sound.useNative({
        media: nativeMedia,
        setVolume: (volume, muted) => invoke("player_volume", { volume, muted }).catch(() => {}),
      });
      audio = nativeMedia;
      await listen("player", (event) => onNativeStatus(event.payload));
      await listen("player-error", (event) => toast(String(event.payload)));
    }
    const snap = await invoke("player_snapshot").catch(() => null);
    // Volume et son coupé : ceux du cœur, qui ont pu changer pendant que la page était fermée.
    if (snap && typeof snap.volume === "number") {
      Sound.adopt(snap.volume, !!snap.muted);
      // Le curseur et les réglages suivent, sinon le prochain réglage renverrait l'ancien volume.
      state.settings.volume = Math.round(snap.volume * 100);
      const slider = el("volume");
      if (slider) {
        slider.value = String(state.settings.volume);
        slider.style.setProperty("--p", `${state.settings.volume}%`);
      }
    } else {
      Sound.configure(state.settings);
    }
    refreshToggles();
    if (!snap || !snap.status.id) return false;
    state.queue = snap.ids;
    const at = snap.ids[snap.status.index] === snap.status.id ? snap.status.index : snap.ids.indexOf(snap.status.id);
    state.queueIndex = Math.max(0, at);
    state.shuffle = snap.shuffle;
    state.repeat = snap.repeat;
    if (typeof snap.status.seq === "number") lastSeq = snap.status.seq;
    nativeMedia.update(snap.status);
    refreshToggles();
    renderAll();
    return true;
  } finally {
    nativeAdopting = false;
    clearTimeout(queueTimer);
    queueTimer = null;
    pushedQueue = queueKey();
  }
}

/// Lecture ou pause affichées dans les listes et le carrousel (seulement au changement).
let listPlaying = null;
function renderPlayState() {
  const playing = isPlaying();
  if (playing === listPlaying) return;
  listPlaying = playing;
  renderCarousel();
  renderMinimal();
  renderLibrary();
}

/// Nouvelles du lecteur du cœur : position, lecture/pause, changement de morceau.
function onNativeStatus(s) {
  if (typeof s.seq === "number") {
    if (s.seq <= lastSeq) return;
    lastSeq = s.seq;
  }
  const hadDuration = nativeMedia.duration;
  nativeMedia.update(s);
  if (nativeExpect) {
    if (s.id === nativeExpect.id || performance.now() > nativeExpect.until) nativeExpect = null;
    else if (!s.changed) return;
  }
  // Le morceau du cœur fait foi : la page s'y recale, changement annoncé ou non.
  if (s.id && currentId() !== s.id) {
    const i = state.queue[s.index] === s.id ? s.index : state.queue.indexOf(s.id);
    if (i >= 0) {
      state.queueIndex = i;
      renderAll();
    }
  } else if (s.changed) {
    renderAll();
  }
  if (s.id && !(hadDuration > 0) && nativeMedia.duration > 0) learnDuration(nativeMedia, s.id);
  updateProgress();
  renderPlayState();
}

/// La file envoyée au cœur en dernier : il ne la reçoit que si elle a changé.
let pushedQueue = "";
let queueTimer = null;

function queueKey() {
  return `${state.queue.length}|${state.queue.join("\n")}|${state.shuffle}|${state.repeat}`;
}

function sendNativeQueue() {
  queueTimer = null;
  const key = queueKey();
  if (key === pushedQueue) return;
  pushedQueue = key;
  invoke("player_queue", {
    ids: state.queue,
    index: Math.max(0, state.queueIndex),
    shuffle: state.shuffle,
    repeat: state.repeat,
  }).catch(() => {});
}

/// File modifiée (ajout, retrait, ordre, aléatoire, répétition) : le cœur suit.
function syncNativeQueue() {
  if (!NATIVE || nativeAdopting) return;
  clearTimeout(queueTimer);
  queueTimer = setTimeout(sendNativeQueue, 80);
}

/// Une file en attente d'envoi part tout de suite (avant Suivant, Lecture…).
function flushNativeQueue() {
  if (!NATIVE || nativeAdopting || !queueTimer) return;
  clearTimeout(queueTimer);
  sendNativeQueue();
}

/// Sous Linux, le lecteur audio de WebKitGTK n'accepte pas asset:// : on lui donne
/// l'adresse file:// du morceau. Ailleurs (WebView2), le protocole d'asset fonctionne.
const LINUX_WEBKIT = /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent) && !window.chrome;

// Sous Windows, les fichiers viennent d'une autre origine (asset) : le traitement du son
// et la capture d'images de vidéo demandent le CORS, que le protocole d'asset accorde.
if (!LINUX_WEBKIT) Sound.players.forEach((p) => (p.crossOrigin = "anonymous"));

/// Durée inconnue des tags (fichier sans en-tête complet) : on garde celle du lecteur.
function learnDuration(media, id = null) {
  const track = id ? trackById(id) : currentTrack();
  if (!track || track.duration_ms > 0 || !isFinite(media.duration) || media.duration <= 0) return;
  track.duration_ms = Math.round(media.duration * 1000);
  invoke("set_duration", { id: track.id, durationMs: track.duration_ms }).catch(() => {});
  libraryVersion += 1;
  renderLibrary();
  renderQueue();
}

function startPlayback() {
  Sound.resume();
  audio.play().catch(() => {});
}

/// Notification du système pour un nouveau morceau, quand Écho n'est pas au premier plan.
function notifyTrack(track) {
  if (NATIVE || !state.settings.notify || document.hasFocus()) return;
  invoke("notify_track", { title: track.title, body: track.artist || "Écho" }).catch(() => {});
}

/// Morceaux muets sautés d'affilée : au-delà de la taille de la file, on s'arrête.
let silentSkips = 0;

function mediaSrc(path) {
  if (!LINUX_WEBKIT) return convertFileSrc(path);
  return "file://" + path.split("/").map(encodeURIComponent).join("/");
}

function isPlaying() {
  const track = currentTrack();
  if (!track) return false;
  return !audio.paused;
}

function playFromList(index) {
  state.queue = [...state.listOrder];
  state.queueIndex = index;
  playCurrent();
}

function playFromResults(id, ids) {
  state.queue = [...ids];
  state.queueIndex = Math.max(0, ids.indexOf(id));
  playCurrent();
}

function buildQueueFromLibrary(startId) {
  const list = visibleTracks().map((t) => t.id);
  if (!list.includes(startId)) list.unshift(startId);
  state.queue = list;
  state.queueIndex = list.indexOf(startId);
}




function playCurrent({ autoplay = true } = {}) {
  const track = currentTrack();
  if (!track) return;
  if (NATIVE) {
    // Le cœur lit, enchaîne (et saute lui-même les vidéos muettes), prévient le
    // système et retient la position.
    const resume = state.resuming && state.resuming.id === track.id && state.settings.remember_position ? state.resuming.position : null;
    state.resuming = null;
    nativeMedia.update({ playing: autoplay, position_ms: resume || 0, duration_ms: track.duration_ms || 0 });
    // Les nouvelles de l'ancien morceau encore en route ne doivent pas faire revenir la page.
    nativeExpect = { id: track.id, until: performance.now() + 1500 };
    clearTimeout(queueTimer);
    queueTimer = null;
    pushedQueue = queueKey();
    invoke("player_play", {
      ids: state.queue,
      index: Math.max(0, state.queueIndex),
      autoplay,
      startMs: resume,
      shuffle: state.shuffle,
      repeat: state.repeat,
    }).catch(() => {});
    renderAll();
    return;
  }
  if (track.has_audio === false) {
    // Vidéo téléchargée sans son : rien à écouter, on passe à la suivante.
    toast(`« ${track.title} » n'a pas de piste son (vidéo seule).`);
    if (autoplay && silentSkips < state.queue.length) {
      silentSkips += 1;
      setTimeout(() => step(1, true), 0);
    }
    return;
  }
  silentSkips = 0;
  Sound.cancelFade();
  const token = ++playToken;
  const begin = () => {
    // Un autre morceau a été lancé entre-temps : celui-ci n'est plus d'actualité.
    if (token !== playToken) return;
    audio = Sound.element;
    audio.src = playableSrc(track);
    Sound.beginTrack(track.id);
    if (autoplay) {
      startPlayback();
      notifyTrack(track);
    }
    if (state.resuming && state.resuming.id === track.id && state.settings.remember_position) {
      const target = state.resuming.position;
      state.resuming = null;
      const media = audio;
      const apply = () => {
        if (isFinite(media.duration) && media.duration > 0) {
          media.currentTime = Math.min(target / 1000, Math.max(0, media.duration - 1));
        }
        media.removeEventListener("loadedmetadata", apply);
      };
      media.addEventListener("loadedmetadata", apply);
    }
    // Le suivant est préparé pendant l'écoute : il démarrera sans attendre.
    prepareAudio(trackById(state.queue[(state.queueIndex + 1) % Math.max(1, state.queue.length)]));
  };
  if (needsAudioCopy(track)) prepareAudio(track).then(begin);
  else begin();
  pushNowPlaying(track);
  pushPlayback(true);
  renderAll();
}

/// Linux : un clip est lu par sa piste audio seule (copie préparée par le cœur), sinon
/// le lecteur de WebKit décoderait aussi toute la vidéo, pour rien.
const audioPaths = new Map();
let playToken = 0;

function needsAudioCopy(track) {
  return !NATIVE && LINUX_WEBKIT && !!track && track.video && track.has_audio !== false && !audioPaths.has(track.id);
}

function playableSrc(track) {
  const copy = LINUX_WEBKIT && track.video ? audioPaths.get(track.id) : null;
  return mediaSrc(copy || track.path);
}

/// Copies en cours de préparation : une seule demande par morceau.
const audioPending = new Map();

function prepareAudio(track) {
  if (!needsAudioCopy(track)) return Promise.resolve();
  if (!audioPending.has(track.id)) {
    const job = invoke("audio_source", { id: track.id })
      .then((path) => {
        if (path && path !== track.path) audioPaths.set(track.id, path);
      })
      .catch(() => {})
      .finally(() => audioPending.delete(track.id));
    audioPending.set(track.id, job);
  }
  return audioPending.get(track.id);
}

function togglePlay() {
  const track = currentTrack();
  // Le cœur joue un morceau qui a quitté la bibliothèque (rescan) : pause quand même.
  if (!track && NATIVE && nativeMedia.id) {
    if (audio.paused) startPlayback();
    else audio.pause();
    return;
  }
  if (!track) {
    if (state.listOrder.length) playFromList(0);
    return;
  }
  if (audio.paused) startPlayback();
  else audio.pause();
}

/// Fondu enchaîné : quelques secondes avant la fin, le morceau suivant démarre
/// sur l'autre lecteur pendant que celui-ci s'éteint.
function maybeCrossfade() {
  const seconds = state.settings.crossfade || 0;
  if (NATIVE || !seconds || Sound.fading || audio.paused || state.repeat === "one") return;
  const duration = audio.duration;
  if (!isFinite(duration) || duration < seconds * 3 || duration - audio.currentTime > seconds) return;
  const i = EchoLogic.nextIndex(playContext(), true);
  if (i < 0 || i === state.queueIndex) return;
  const next = trackById(state.queue[i]);
  if (!next || next.has_audio === false) return;
  if (state.shuffle) shuffleBag.history.push(state.queueIndex);
  state.queueIndex = i;
  Sound.other.src = playableSrc(next);
  audio = Sound.crossfade(seconds);
  prepareAudio(trackById(state.queue[(i + 1) % state.queue.length]));
  Sound.beginTrack(next.id);
  startPlayback();
  notifyTrack(next);
  pushNowPlaying(next);
  pushPlayback(true);
  renderAll();
}

/// Mémoire de l'aléatoire : repart de zéro dès que la file change.
let shuffleBag = EchoLogic.freshBag(null);

function playContext() {
  if (shuffleBag.queue !== state.queue) shuffleBag = EchoLogic.freshBag(state.queue);
  return { n: state.queue.length, index: state.queueIndex, shuffle: state.shuffle, repeat: state.repeat, bag: shuffleBag };
}

function step(delta, auto = false) {
  if (NATIVE && state.queue.length) {
    flushNativeQueue();
    invoke("player_step", { delta }).catch(() => {});
    return;
  }
  if (state.queue.length === 0) {
    if (state.listOrder.length) playFromList(0);
    return;
  }
  const ctx = playContext();
  if (delta > 0) {
    const i = EchoLogic.nextIndex(ctx, auto);
    if (i < 0) {
      audio.pause();
      return;
    }
    if (i === state.queueIndex) {
      // « Un titre » : on rejoue sans recharger le fichier.
      audio.currentTime = 0;
      startPlayback();
      return;
    }
    if (state.shuffle) shuffleBag.history.push(state.queueIndex);
    state.queueIndex = i;
  } else {
    // Précédent, au milieu d'un titre : on revient d'abord au début, comme les autres lecteurs.
    if (!auto && mediaEl().currentTime > 3) {
      mediaEl().currentTime = 0;
      return;
    }
    state.queueIndex = EchoLogic.prevIndex(ctx);
  }
  playCurrent();
}

/* ---------------- avancement ---------------- */

let lastProgressPush = 0;
let lastSavedPosition = 0;

/// Dernier état affiché sur les boutons lecture (null : jamais dessinés).
let lastPlayingShown = null;

/// Regroupe les événements de défilement : une liste n'est redessinée qu'une fois par image.
function onFrame(draw) {
  let queued = false;
  return () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      draw();
    });
  };
}

/// Texte réécrit seulement s'il change (rien à repeindre sinon).
function setText(node, text) {
  if (node && node.textContent !== text) node.textContent = text;
}

/// Barre fine (compacte, minimaliste) : elle glisse d'une seconde à l'autre, mais saute
/// sans animation en arrière ou sur un grand écart (nouveau morceau, saut).
function setBar(bar, ratio) {
  if (!bar) return;
  const fill = bar.firstElementChild;
  const next = ratio * 100;
  const prev = parseFloat(fill.style.width) || 0;
  fill.style.transition = next >= prev && next - prev < 3 ? "" : "none";
  fill.style.width = `${next}%`;
}

function updateProgress() {
  const track = currentTrack();
  const media = audio;
  const dur = media.duration || 0;
  const ratio = !state.seeking && isFinite(dur) && dur > 0 ? media.currentTime / dur : null;
  const now = media.currentTime || 0;

  if (ratio !== null) {
    const seek = el("seek");
    if (seek) {
      seek.value = String(Math.round(ratio * 1000));
      seek.style.setProperty("--p", `${(ratio * 100).toFixed(2)}%`);
    }
    setBar(el("comp-bar"), ratio);
    setBar(el("min-bar"), ratio);
  }

  const nowText = formatTime(now * 1000);
  const totalText = formatTime((dur || 0) * 1000);
  // Pendant un glisser de la barre, le temps affiché est celui du curseur.
  if (!state.seeking) ["time-now", "comp-now", "min-now-time"].forEach((id) => setText(el(id), nowText));
  ["time-total", "comp-total", "min-total-time"].forEach((id) => setText(el(id), totalText));

  const playing = !media.paused;
  // Les boutons ne changent qu'au passage lecture ↔ pause, pas à chaque seconde.
  if (playing !== lastPlayingShown) {
    lastPlayingShown = playing;
    document.querySelectorAll(".ctl-play, .play-big").forEach((b) => b.classList.toggle("is-playing", playing));
    // Infobulle et nom accessible suivent l'état, sur tous les boutons lecture.
    const label = playing ? "Pause" : "Lecture";
    ["btn-play", "btn-play-big", "comp-play", "min-play"].forEach((id) => {
      const b = el(id);
      if (!b) return;
      b.title = label;
      b.setAttribute("aria-label", label);
    });
    const compPlay = el("comp-play");
    if (compPlay) {
      compPlay.innerHTML = icon(playing ? "pause" : "play", 20);
      compPlay.classList.toggle("is-playing", playing);
    }
    const minPlay = el("min-play");
    if (minPlay) minPlay.innerHTML = icon(playing ? "pause" : "play", 18);
  }

  if (playing) {
    const stamp = Date.now();
    if (stamp - lastProgressPush > 5000) {
      lastProgressPush = stamp;
      pushPlayback();
    }
    if (stamp - lastSavedPosition > 15000) {
      lastSavedPosition = stamp;
      saveProgress(true);
    }
  }
}

function seekToRatio(ratio) {
  const media = mediaEl();
  const dur = media.duration || 0;
  if (dur > 0) media.currentTime = Math.min(Math.max(0, ratio), 1) * dur;
}

function bindBar(node) {
  if (!node) return;
  // La barre se comporte comme un curseur : elle reçoit le focus et les flèches.
  node.tabIndex = 0;
  node.setAttribute("role", "slider");
  node.setAttribute("aria-label", "Position dans le morceau");
  node.addEventListener("click", (e) => {
    const rect = node.getBoundingClientRect();
    seekToRatio((e.clientX - rect.left) / rect.width);
  });
  node.addEventListener("keydown", (e) => {
    if (e.code !== "ArrowLeft" && e.code !== "ArrowRight") return;
    e.preventDefault();
    e.stopPropagation();
    const media = mediaEl();
    const dur = media.duration || 0;
    if (!dur) return;
    seekToRatio(media.currentTime / dur + (e.code === "ArrowRight" ? 0.05 : -0.05));
  });
}

/* ---------------- touches média ---------------- */

let lastPositionPush = 0;

function pushNowPlaying(track) {
  if (!track) {
    invoke("set_now_playing", { title: "", artist: "", album: "", cover: null, durationMs: 0 }).catch(() => {});
    return;
  }
  invoke("set_now_playing", {
    title: track.title,
    artist: track.artist || "",
    album: track.album || "",
    cover: track.cover || null,
    durationMs: track.duration_ms || 0,
  }).catch(() => {});
}

function pushPlayback(force = false) {
  if (NATIVE) return;
  const track = currentTrack();
  const media = audio;
  const now = Date.now();
  if (!force && now - lastPositionPush < 5000) return;
  lastPositionPush = now;
  invoke("set_playback", {
    playing: !media.paused && !!track,
    positionMs: Math.round((media.currentTime || 0) * 1000),
  }).catch(() => {});
}

function saveProgress(force = false) {
  if (NATIVE) return;
  const track = currentTrack();
  const media = audio;
  if (!state.settings.resume_last && !force) return;
  invoke("save_progress", {
    trackId: track ? track.id : null,
    positionMs: Math.round((media.currentTime || 0) * 1000),
    playing: !media.paused && !!track,
  }).catch(() => {});
}

async function bindMediaKeys() {
  await listen("media-control", (event) => {
    const media = mediaEl();
    // Les actions avec une valeur arrivent sous la forme « action:millisecondes ».
    const [action, raw] = String(event.payload).split(":");
    const ms = Number(raw);
    if (action === "set-position" && isFinite(ms)) {
      media.currentTime = Math.min(media.duration || 0, Math.max(0, ms / 1000));
    } else if (action === "seek-by" && isFinite(ms)) {
      media.currentTime = Math.min(media.duration || 0, Math.max(0, media.currentTime + ms / 1000));
    } else if (action === "quit") {
      saveProgress(true);
      invoke("quit_app").catch(() => {});
      return;
    }
    switch (action) {
      case "play":
        startPlayback();
        break;
      case "pause":
        media.pause();
        break;
      case "toggle":
        togglePlay();
        break;
      case "stop":
        media.pause();
        media.currentTime = 0;
        break;
      case "next":
        step(1);
        break;
      case "previous":
        step(-1);
        break;
      case "seek-forward":
        media.currentTime = Math.min(media.duration || 0, media.currentTime + state.settings.skip_seconds);
        break;
      case "seek-backward":
        media.currentTime = Math.max(0, media.currentTime - state.settings.skip_seconds);
        break;
      default:
        break;
    }
    pushPlayback(true);
  });
}

/* ============================================================
   Affichage général
   ============================================================ */

/// Dernier morceau affiché : une pochette ne s'anime que quand le morceau change.
let lastNowId = null;
let lastCarouselId = null;

/// Rejoue une animation CSS sur un élément (la classe est retirée puis remise).
function playAnim(node, cls) {
  // Élément caché : rien à animer (et l'animation se rejouerait à son retour).
  if (!node || !node.getClientRects().length) return;
  node.classList.remove(cls);
  void node.offsetWidth;
  node.classList.add(cls);
  // Classe retirée à la fin : sinon l'animation repart quand l'élément réapparaît.
  node.addEventListener("animationend", () => node.classList.remove(cls), { once: true });
}

function renderNowPlaying() {
  const track = currentTrack();
  if ((track ? track.id : null) !== lastNowId) {
    // Premier affichage (démarrage, page rouverte) : pas d'animation de changement.
    const animate = lastNowId !== null;
    lastNowId = track ? track.id : null;
    if (lyricsOpen) loadLyrics();
    if (animate) {
      ["listen-cover", "mini-cover", "min-mini-cover"].forEach((id) => playAnim(el(id), "swap"));
      ["listen-title", "listen-artist", "comp-title", "comp-artist", "mini-title", "mini-artist", "min-title", "min-artist"]
        .forEach((id) => playAnim(el(id), "swap-text"));
    }
  }
  // « Modifier » sans morceau ne ferait rien : le bouton le dit.
  const edit = el("btn-edit-listen");
  if (edit) edit.disabled = !track;
  const cover = el("listen-cover");
  const miniCover = el("mini-cover");
  const minCover = el("min-mini-cover");

  const title = track ? track.title : "Rien en cours";
  const artist = track ? track.artist || "—" : "Choisis un morceau dans ta bibliothèque";

  el("listen-title").textContent = title;
  el("listen-artist").textContent = artist;
  el("mini-title").textContent = track ? track.title : "Aucun morceau";
  el("mini-artist").textContent = track ? track.artist || "—" : "—";
  el("comp-title").textContent = title;
  el("comp-artist").textContent = track ? `${track.artist || "—"}${track.album ? " · " + track.album : ""}` : "Choisis un morceau dans ta bibliothèque";
  el("min-title").textContent = track ? track.title : "Aucun morceau";
  el("min-artist").textContent = track ? track.artist || "—" : "—";

  if (track) {
    fillCover(cover, track);
    fillCover(miniCover, track);
    fillCover(minCover, track);
  } else {
    [cover, miniCover, minCover].forEach((node) => {
      delete node.dataset.shown;
      delete node.dataset.coverOf;
      node.style.background = "";
      node.innerHTML = "";
    });
  }
}

let bannerDismissed = null;

function renderAll() {
  const audios = audioTracks().length;
  const count = el("track-count");
  // Pendant une analyse, la pastille parle ; le compteur garde son dernier nombre.
  setText(count, countLabel(audios));

  el("empty").hidden = state.folders.length > 0;
  const banner = el("banner");
  const missingKey = state.foldersMissing.join("\n");
  // Une fois fermé, le bandeau ne revient que si la liste des dossiers manquants change.
  if (state.foldersMissing.length && missingKey !== bannerDismissed) {
    banner.hidden = false;
    el("banner-text").textContent =
      state.foldersMissing.length === 1
        ? `Dossier introuvable : ${state.foldersMissing[0]}`
        : `${state.foldersMissing.length} dossiers sont introuvables.`;
  } else {
    banner.hidden = true;
  }

  renderLibrary();
  renderQueue();
  renderNowPlaying();
  renderCarousel();
  renderMinimal();
  updateProgress();
}

function switchView(view) {
  state.view = view;
  document.querySelectorAll(".nav-item").forEach((b) => b.classList.toggle("is-active", b.dataset.view === view));
  document.querySelectorAll(".view").forEach((v) => v.classList.toggle("is-active", v.id === `view-${view}`));
  syncSettingsNav();
  if (view === "library") renderLibrary();
  if (view === "playlists") renderPlaylists();
}

/* ---------------- surfaces superposées ---------------- */

const HOMES = new Map();

/// Remet un élément à sa place d'origine dans la page.
function returnHome(node) {
  const home = HOMES.get(node);
  if (home && home.isConnected) {
    home.appendChild(node);
    return;
  }
  // Pas d'emplacement connu : on ne supprime jamais un panneau de l'interface.
  if (node.id === "settings-panel") el("settings-host-normal").appendChild(node);
  else if (node.id === "library-panel") el("view-library").appendChild(node);
  else if (node.id === "queue") el("view-listen").appendChild(node);
  else if (node.id === "playlists-panel") el("view-playlists").appendChild(node);
  else node.remove();
}

function openSheet(node, title, { flush = false } = {}) {
  if (node && !HOMES.has(node) && node.parentElement) HOMES.set(node, node.parentElement);
  const body = el("sheet-body");
  // Un panneau déjà ouvert retourne chez lui avant d'être remplacé, sinon il serait perdu.
  const previous = body.firstElementChild;
  if (previous && previous !== node) returnHome(previous);
  el("sheet-title").textContent = title;
  body.classList.toggle("flush", flush);
  if (node) body.replaceChildren(node);
  el("sheet").hidden = false;
  requestAnimationFrame(() => {
    if (node && node.id === "library-panel") renderLibrary();
  });
}

function closeSheet() {
  const body = el("sheet-body");
  const node = body.firstElementChild;
  if (node) returnHome(node);
  el("sheet").hidden = true;
}

/// « Aucun morceau », « 1 morceau », « 12 morceaux ».
function countLabel(n, zero = "Aucun morceau") {
  return n ? `${n} morceau${n > 1 ? "x" : ""}` : zero;
}

function toast(text) {
  let node = el("toast");
  if (!node) {
    node = document.createElement("div");
    node.id = "toast";
    // Au-dessus de tout (paroles, éditeur, fenêtres superposées) et lu par les lecteurs d'écran.
    node.className = "banner toast";
    node.setAttribute("role", "status");
    node.setAttribute("aria-live", "polite");
    document.body.appendChild(node);
  }
  node.textContent = text;
  node.hidden = false;
  clearTimeout(node._timer);
  // Le temps de lire : un message long reste affiché plus longtemps.
  node._timer = setTimeout(() => (node.hidden = true), Math.max(3200, text.length * 55));
}

/// Annule la boîte de dialogue ouverte (Échap) : renvoie « non ».
let modalCancel = null;

function ask(title, text, options, { input = null } = {}) {
  return new Promise((resolve) => {
    el("modal-title").textContent = title;
    el("modal-text").textContent = text;
    const field = el("modal-input");
    field.hidden = input === null;
    field.value = input ?? "";
    field.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        finish(true);
      }
    };
    const actions = el("modal-actions");
    actions.replaceChildren();
    const finish = (value) => {
      el("modal").hidden = true;
      modalCancel = null;
      // Avec un champ : la valeur saisie si l'on valide, null sinon.
      resolve(input === null ? value : value ? field.value.trim() : null);
    };
    options.forEach((option) => {
      const btn = document.createElement("button");
      btn.className = option.primary ? "primary-btn" : "ghost-btn";
      btn.textContent = option.label;
      btn.addEventListener("click", () => finish(option.value));
      actions.appendChild(btn);
    });
    modalCancel = () => finish(false);
    el("modal").hidden = false;
    if (input !== null) {
      field.focus();
      field.select();
    }
  });
}

/* ---------------- modifier un morceau ---------------- */

/// Morceau en cours d'édition : valeurs d'origine et choix faits dans la fenêtre.
let editing = null;

const DEFAULT_COVER = "linear-gradient(140deg, var(--accent), var(--accent2))";

function paintEditCover(coverName, pickedPath = null) {
  const node = el("edit-cover");
  if (pickedPath) {
    node.style.background = `var(--cover-bg) url("${convertFileSrc(pickedPath)}") center / cover no-repeat`;
    return;
  }
  if (!coverName) {
    node.style.background = DEFAULT_COVER;
    return;
  }
  coverSrc(coverName).then((url) => {
    node.style.background = url ? `var(--cover-bg) url("${url}") center / cover no-repeat` : DEFAULT_COVER;
  });
}

async function openEditor(track) {
  if (!track) return;
  let original = null;
  try {
    original = await invoke("track_original", { id: track.id });
  } catch (err) {
    console.error(err);
  }
  if (!original) {
    toast("Ce morceau n'est plus dans la bibliothèque.");
    return;
  }
  editing = { id: track.id, original, pickedCover: null, clearCover: false };
  el("edit-title").value = track.title || "";
  el("edit-artist").value = track.artist || "";
  el("edit-album").value = track.album || "";
  paintEditCover(track.cover);
  el("edit-dialog").hidden = false;
  el("edit-title").focus();
}

function closeEditor() {
  el("edit-dialog").hidden = true;
  editing = null;
}

function bindEditor() {
  el("btn-edit-listen").addEventListener("click", () => openEditor(currentTrack()));
  el("edit-close").addEventListener("click", closeEditor);
  el("edit-cancel").addEventListener("click", closeEditor);
  el("edit-dialog").addEventListener("click", (e) => {
    if (e.target === el("edit-dialog")) closeEditor();
  });

  el("edit-cover").addEventListener("click", async () => {
    if (!editing) return;
    const picked = await openDialog({
      multiple: false,
      title: "Choisir une image de pochette",
      filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp", "gif"] }],
    }).catch(() => null);
    if (!picked) return;
    editing.pickedCover = typeof picked === "string" ? picked : picked.path;
    editing.clearCover = false;
    // L'image peut venir de n'importe quel dossier : le cœur l'autorise pour l'aperçu.
    await invoke("allow_preview", { path: editing.pickedCover }).catch(() => {});
    paintEditCover(null, editing.pickedCover);
  });

  // Rétablir l'original : on remet les champs et on retire toutes les corrections.
  el("edit-reset").addEventListener("click", () => {
    if (!editing) return;
    el("edit-title").value = editing.original.title;
    el("edit-artist").value = editing.original.artist;
    el("edit-album").value = editing.original.album;
    editing.pickedCover = null;
    editing.clearCover = true;
    paintEditCover(editing.original.cover);
  });

  el("edit-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    if (!editing) return;
    if (!el("edit-title").value.trim()) {
      el("edit-title").focus();
      toast("Le titre ne peut pas être vide.");
      return;
    }
    el("edit-save").disabled = true;
    try {
      const updated = await invoke("edit_track", {
        id: editing.id,
        title: el("edit-title").value,
        artist: el("edit-artist").value,
        album: el("edit-album").value,
        coverPath: editing.pickedCover,
        clearCover: editing.clearCover,
      });
      const edited = editing.id;
      closeEditor();
      replaceTrack(updated);
      // Si le morceau modifié joue, la barre des médias du système suit la correction.
      if (currentId() === edited) pushNowPlaying(currentTrack());
      toast("Morceau mis à jour.");
    } catch (err) {
      toast(String(err));
    } finally {
      el("edit-save").disabled = false;
    }
  });

  // Clic droit sur n'importe quel morceau (liste, file, playlist, vue minimaliste) : son menu.
  document.addEventListener("contextmenu", (e) => {
    const node = e.target.closest("[data-track-id], [data-min-id]");
    if (!node) return;
    const track = trackById(node.dataset.trackId || node.dataset.minId);
    if (!track) return;
    e.preventDefault();
    openTrackMenu(track, e.clientX, e.clientY, { playlist: node.dataset.playlist || null });
  });
}

/* ---------------- menu d'un morceau ---------------- */

function closeTrackMenu() {
  const menu = el("ctx-menu");
  if (menu) menu.hidden = true;
}

/// Insère un morceau dans la file : juste après celui en cours, ou à la fin.
function enqueue(track, next) {
  if (!state.queue.length) {
    state.queue = [track.id];
    state.queueIndex = 0;
    playCurrent();
    return;
  }
  const queue = [...state.queue];
  if (next) queue.splice(state.queueIndex + 1, 0, track.id);
  else queue.push(track.id);
  state.queue = queue;
  renderQueue();
  renderCarousel();
  toast(next ? "Sera lu juste après." : "Ajouté à la file.");
}

function openTrackMenu(track, x, y, { playlist = null } = {}) {
  const menu = el("ctx-menu");
  const item = (label, iconName, action, extra = "") =>
    `<button role="menuitem" class="ctx-item${extra}" data-act="${action}">${icon(iconName, 15)}<span>${label}</span></button>`;

  const main = () => {
    menu.innerHTML = `
      <div class="ctx-head"><div class="ctx-title">${escapeHtml(track.title)}</div><div class="ctx-sub">${escapeHtml(track.artist || "—")}</div></div>
      ${item("Lire maintenant", "play", "play")}
      ${item("Lire ensuite", "next", "next")}
      ${item("Ajouter à la file", "queue", "queue")}
      <div class="ctx-sep"></div>
      ${item("Ajouter à une playlist", "playlist", "playlists", " has-sub")}
      ${playlist ? item("Retirer de cette playlist", "close", "remove") : ""}
      <div class="ctx-sep"></div>
      ${item("Modifier le morceau…", "sliders", "edit")}`;
  };
  const lists = () => {
    menu.innerHTML = `
      <button class="ctx-item ctx-back" data-act="back">${icon("arrow_left", 15)}<span>Ajouter à une playlist</span></button>
      <div class="ctx-sep"></div>
      ${state.playlists
        .map((p) => {
          const has = p.tracks.includes(track.id);
          return `<button role="menuitemcheckbox" aria-checked="${has}" class="ctx-item" data-act="toggle" data-pl="${escapeHtml(p.id)}">${icon(has ? "check" : "playlist", 15)}<span>${escapeHtml(p.name)}</span></button>`;
        })
        .join("")}
      ${item("Nouvelle playlist…", "plus", "new")}`;
  };
  main();

  menu.onclick = async (e) => {
    const btn = e.target.closest("[data-act]");
    if (!btn) return;
    e.stopPropagation();
    switch (btn.dataset.act) {
      case "play":
        closeTrackMenu();
        buildQueueFromLibrary(track.id);
        playCurrent();
        break;
      case "next":
        closeTrackMenu();
        enqueue(track, true);
        break;
      case "queue":
        closeTrackMenu();
        enqueue(track, false);
        break;
      case "playlists":
        lists();
        break;
      case "back":
        main();
        break;
      case "toggle": {
        const list = state.playlists.find((p) => p.id === btn.dataset.pl);
        if (!list) return;
        const had = list.tracks.includes(track.id);
        list.tracks = had ? list.tracks.filter((id) => id !== track.id) : [...list.tracks, track.id];
        await savePlaylists();
        toast(had ? `Retiré de « ${list.name} ».` : `Ajouté à « ${list.name} ».`);
        lists();
        break;
      }
      case "new": {
        closeTrackMenu();
        const created = await createPlaylist([track.id]);
        if (created) toast(`Ajouté à « ${created.name} ».`);
        break;
      }
      case "remove": {
        closeTrackMenu();
        const list = state.playlists.find((p) => p.id === playlist);
        if (!list) return;
        list.tracks = list.tracks.filter((id) => id !== track.id);
        await savePlaylists();
        break;
      }
      case "edit":
        closeTrackMenu();
        openEditor(track);
        break;
    }
  };

  menu.hidden = false;
  // Le menu reste dans la fenêtre, même ouvert près d'un bord.
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, innerWidth - rect.width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, innerHeight - rect.height - 8))}px`;
  playAnim(menu, "menu-in");
  const first = menu.querySelector(".ctx-item");
  if (first) first.focus({ preventScroll: true });
}

/* ---------------- playlists ---------------- */

function newPlaylistId() {
  return `pl-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

async function savePlaylists() {
  try {
    state.playlists = await invoke("save_playlists", { playlists: state.playlists });
  } catch (err) {
    toast(`Playlist non enregistrée : ${err}`);
  }
  renderPlaylists();
}

async function createPlaylist(tracks = []) {
  const name = await ask("Nouvelle playlist", "Donne-lui un nom.", [
    { label: "Annuler", value: false },
    { label: "Créer", value: true, primary: true },
  ], { input: `Playlist ${state.playlists.length + 1}` });
  if (!name) return null;
  const list = { id: newPlaylistId(), name, tracks };
  state.playlists = [...state.playlists, list];
  state.playlistOpen = list.id;
  await savePlaylists();
  return list;
}

function playlistTracks(list) {
  return list.tracks.map(trackById).filter(Boolean);
}

function playPlaylist(list, { shuffle = false, startId = null } = {}) {
  const tracks = playlistTracks(list).filter((t) => t.has_audio !== false);
  if (!tracks.length) {
    toast("Cette playlist est vide.");
    return;
  }
  state.queue = tracks.map((t) => t.id);
  state.shuffle = shuffle || state.shuffle;
  state.queueIndex = startId ? Math.max(0, state.queue.indexOf(startId)) : shuffle ? Math.floor(Math.random() * tracks.length) : 0;
  refreshToggles();
  playCurrent();
}

function renderPlaylists() {
  const listBox = el("pl-list");
  const detail = el("pl-detail");
  if (!listBox || !detail) return;
  const lists = state.playlists;
  if (!lists.some((p) => p.id === state.playlistOpen)) state.playlistOpen = lists.length ? lists[0].id : null;

  listBox.innerHTML = lists.length
    ? lists
        .map((p) => {
          const count = playlistTracks(p).length;
          return `<button role="listitem" class="pl-item${p.id === state.playlistOpen ? " on" : ""}" data-open="${escapeHtml(p.id)}">
            <span class="pl-mark">${icon("playlist", 16)}</span>
            <span class="pl-name">${escapeHtml(p.name)}</span>
            <span class="pl-count">${count}</span>
          </button>`;
        })
        .join("")
    : `<div class="hint pl-empty">Aucune playlist pour l'instant.</div>`;

  const open = lists.find((p) => p.id === state.playlistOpen);
  if (!open) {
    detail.innerHTML = `<div class="pl-intro">
      <div class="pl-intro-mark">${icon("playlist", 28)}</div>
      <div class="album-title">Tes playlists</div>
      <div class="album-meta">Crée une playlist, puis fais un clic droit sur un morceau → « Ajouter à une playlist ».</div>
    </div>`;
    return;
  }
  const tracks = playlistTracks(open);
  const total = tracks.reduce((sum, t) => sum + (t.duration_ms || 0), 0);
  detail.innerHTML = `
    <div class="pl-head">
      <div class="pl-cover" id="pl-cover"></div>
      <div class="pl-info">
        <div class="listen-kicker">Playlist</div>
        <div class="pl-title">${escapeHtml(open.name)}</div>
        <div class="album-meta">${countLabel(tracks.length)}${total ? " · " + formatTime(total) : ""}</div>
        <div class="listen-actions">
          <button class="play-big" data-pl-act="play" aria-label="Lire la playlist" title="Lire">${icon("play", 20)}</button>
          <button class="ghost-btn" data-pl-act="shuffle">${icon("shuffle", 15)}Aléatoire</button>
          <button class="ghost-btn" data-pl-act="rename">${icon("sliders", 15)}Renommer</button>
          <button class="ghost-btn danger" data-pl-act="delete">${icon("close", 15)}Supprimer</button>
        </div>
      </div>
    </div>
    <div class="pl-tracks">${
      tracks.length
        ? tracks
            .map(
              (t, i) => `<div class="pl-row${t.id === currentId() ? " is-current" : ""}" data-track-id="${escapeHtml(t.id)}" data-playlist="${escapeHtml(open.id)}">
                <span class="trk-n">${t.id === currentId() && isPlaying() ? `<span class="playing">${ICONS.waveform}</span>` : i + 1}</span>
                <div class="thumb"></div>
                <span class="pl-row-text"><span class="row-title">${escapeHtml(t.title)}</span><span class="row-artist">${escapeHtml(t.artist || "—")}</span></span>
                <span class="row-time">${t.duration_ms ? formatTime(t.duration_ms) : "—"}</span>
                <button class="icon-btn pl-remove" data-remove="${escapeHtml(t.id)}" aria-label="Retirer de la playlist" title="Retirer">${icon("close", 14)}</button>
              </div>`
            )
            .join("")
        : `<div class="hint pl-empty">Playlist vide : clic droit sur un morceau de la bibliothèque → « Ajouter à une playlist ».</div>`
    }</div>`;
  fillCover(el("pl-cover"), tracks.find((t) => t.cover) || tracks[0] || null, false);
  detail.querySelectorAll(".pl-row").forEach((row, i) => fillCover(row.querySelector(".thumb"), tracks[i]));
}

function bindPlaylists() {
  el("btn-new-playlist").addEventListener("click", () => createPlaylist());
  el("pl-list").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-open]");
    if (!btn) return;
    state.playlistOpen = btn.dataset.open;
    renderPlaylists();
  });
  el("pl-detail").addEventListener("click", async (e) => {
    const open = state.playlists.find((p) => p.id === state.playlistOpen);
    if (!open) return;
    const remove = e.target.closest("[data-remove]");
    if (remove) {
      e.stopPropagation();
      open.tracks = open.tracks.filter((id) => id !== remove.dataset.remove);
      await savePlaylists();
      return;
    }
    const action = e.target.closest("[data-pl-act]");
    if (action) {
      switch (action.dataset.plAct) {
        case "play":
          playPlaylist(open);
          break;
        case "shuffle":
          playPlaylist(open, { shuffle: true });
          break;
        case "rename": {
          const name = await ask("Renommer la playlist", "Nouveau nom :", [
            { label: "Annuler", value: false },
            { label: "Renommer", value: true, primary: true },
          ], { input: open.name });
          if (name) {
            open.name = name;
            await savePlaylists();
          }
          break;
        }
        case "delete": {
          const ok = await ask(`Supprimer « ${open.name} » ?`, "Les morceaux restent dans ta bibliothèque.", [
            { label: "Annuler", value: false },
            { label: "Supprimer", value: true, primary: true },
          ]);
          if (ok) {
            state.playlists = state.playlists.filter((p) => p.id !== open.id);
            await savePlaylists();
          }
          break;
        }
      }
      return;
    }
    const row = e.target.closest(".pl-row");
    if (row) playPlaylist(open, { startId: row.dataset.trackId });
  });
}

/* ---------------- pochettes tirées des vidéos ---------------- */

/// Vidéos déjà essayées pendant cette session (réussite ou non).
const frameTried = new Set();
let framesRunning = false;
let framesTimer = null;

function scheduleFrames() {
  clearTimeout(framesTimer);
  framesTimer = setTimeout(extractFrames, 1500);
}

/// Image carrée prise au centre de la vidéo ; « dark » si elle est presque noire.
function squareFrame(video) {
  const w = video.videoWidth;
  const h = video.videoHeight;
  const side = Math.min(w, h);
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 480;
  const g = canvas.getContext("2d");
  g.drawImage(video, (w - side) / 2, (h - side) / 2, side, side, 0, 0, 480, 480);
  const probe = document.createElement("canvas");
  probe.width = probe.height = 12;
  const pg = probe.getContext("2d");
  pg.drawImage(canvas, 0, 0, 12, 12);
  const px = pg.getImageData(0, 0, 12, 12).data;
  let light = 0;
  for (let i = 0; i < px.length; i += 4) light += px[i] + px[i + 1] + px[i + 2];
  const dark = light / (px.length / 4) / 3 < 18;
  const data = atob(canvas.toDataURL("image/jpeg", 0.86).split(",")[1]);
  const bytes = new Array(data.length);
  for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i);
  return { bytes, dark };
}

/// Windows : ouvre la vidéo sans son, va au tiers, et capture une image (évite le noir du début).
/// Résultat : { bytes, missing } — `missing` : le fichier a bien une image (le cœur l'a
/// vu), mais le système ne sait pas la décoder (AV1 sans greffon, par exemple).
/// Rien n'est alors noté : on réessaiera au prochain lancement.
function grabFrame(track) {
  return new Promise((resolve) => {
    const video = document.createElement("video");
    video.className = "frame-grabber";
    video.muted = true;
    video.preload = "auto";
    if (!LINUX_WEBKIT) video.crossOrigin = "anonymous";
    const points = [0.33, 0.5, 0.18, 0.7];
    let attempt = 0;
    let best = null;
    let done = false;
    const finish = (bytes, missing = false) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      video.pause();
      video.removeAttribute("src");
      video.load();
      video.remove();
      resolve({ bytes, missing });
    };
    const timer = setTimeout(() => finish(best), 20000);
    video.addEventListener("loadedmetadata", () => {
      // Image annoncée de 0×0 : il manque le décodeur de ce format vidéo.
      if (!video.videoWidth) return finish(null, true);
      if (!isFinite(video.duration) || video.duration <= 0) return finish(null);
      video.currentTime = video.duration * points[0];
    });
    video.addEventListener("seeked", () => video.play().catch(() => finish(best)));
    video.addEventListener("playing", () =>
      setTimeout(() => {
        if (done) return;
        let shot;
        try {
          shot = squareFrame(video);
        } catch {
          return finish(null);
        }
        best = best || shot.bytes;
        if (!shot.dark) return finish(shot.bytes);
        attempt += 1;
        if (attempt >= points.length) return finish(best);
        video.pause();
        video.currentTime = video.duration * points[attempt];
      }, 450)
    );
    video.addEventListener("error", () => finish(null, true));
    document.body.appendChild(video);
    video.src = mediaSrc(track.path);
  });
}

/// Une vidéo à la fois, en arrière-plan : chaque image trouvée devient une pochette.
async function extractFrames() {
  if (framesRunning || !state.settings || !state.settings.show_covers) return;
  framesRunning = true;
  let missing = 0;
  try {
    for (;;) {
      const track = state.tracks.find((t) => t.video && !t.cover && !t.no_frame && !frameTried.has(t.id));
      if (!track) break;
      frameTried.add(track.id);
      if (LINUX_WEBKIT) {
        // Linux : le cœur décode une seule image avec GStreamer, sans passer par la page.
        const result = await invoke("grab_frame", { id: track.id }).catch(() => null);
        if (result && result.track) replaceTrack(result.track);
        else if (result && result.missing) missing += 1;
      } else {
        const result = await grabFrame(track);
        if (result.bytes) {
          const updated = await invoke("save_frame", { id: track.id, jpeg: result.bytes }).catch(() => null);
          if (updated) replaceTrack(updated);
        } else if (result.missing) {
          missing += 1;
        }
      }
      await new Promise((r) => setTimeout(r, 200));
    }
  } finally {
    framesRunning = false;
  }
  if (missing && !framesHintShown) {
    framesHintShown = true;
    toast(
      `${missing} vidéo${missing > 1 ? "s sont" : " est"} dans un format d'image (souvent AV1) que ton système ne décode pas : ` +
        "installe le greffon dav1d de GStreamer pour avoir leur pochette. Le son, lui, marche."
    );
  }
}

/// Le conseil « décodeur manquant » n'est donné qu'une fois par lancement.
let framesHintShown = false;

/* ---------------- dossiers ---------------- */

async function addFolder() {
  try {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: "Choisir un dossier de musique",
    });
    if (!picked) return;
    const path = typeof picked === "string" ? picked : picked.path;
    const snap = await invoke("add_folder", { path });
    applySnapshot(snap);
    refreshSettingsPanel();
  } catch (err) {
    toast(String(err));
  }
}

async function rescan() {
  const snap = await invoke("rescan");
  applySnapshot(snap);
}

/// Chemins de toutes les pochettes en une fois : au défilement, une nouvelle ligne
/// affiche sa pochette immédiatement au lieu d'un dégradé le temps de la demander.
let coverPrefetch = null;
function prefetchCovers() {
  if (coverPrefetch) return coverPrefetch;
  coverPrefetch = invoke("get_cover_paths")
    .then((paths) => {
      const fresh = [];
      for (const [id, entry] of Object.entries(paths || {})) {
        if (coverUrls.get(id)) continue;
        rememberCover(id, entry);
        fresh.push(thumbUrls.get(id) || coverUrls.get(id));
      }
      // Les toutes premières décodées d'avance (celles visibles au démarrage) : au-delà,
      // des centaines d'images décodées pèseraient lourd en mémoire pour rien.
      fresh.slice(0, 24).forEach((url) => {
        const img = new Image();
        img.decoding = "async";
        img.src = url;
      });
      // Les pochettes affichées prennent leur image sur place : rien n'est reconstruit.
      if (fresh.length) repaintCovers();
    })
    .catch(() => {})
    .finally(() => {
      coverPrefetch = null;
    });
  return coverPrefetch;
}

/// Empreinte de la dernière bibliothèque reçue : un scan qui ne trouve rien de neuf
/// (démarrage, surveillance des dossiers, « Actualiser ») ne redessine rien.
let snapshotSig = "";

function snapshotKey(snapshot) {
  const tracks = (snapshot.tracks || [])
    .map((t) => `${t.id}\u0001${t.cover || ""}\u0001${t.duration_ms}\u0001${t.title}\u0001${t.artist}\u0001${t.album}\u0001${t.has_audio}`)
    .join("\u0002");
  return `${(snapshot.folders || []).join("|")}#${(snapshot.folders_missing || []).join("|")}#${tracks}`;
}

function applySnapshot(snapshot) {
  if (!snapshot) return;
  const sig = snapshotKey(snapshot);
  if (sig === snapshotSig) return;
  snapshotSig = sig;
  state.folders = snapshot.folders || [];
  state.foldersMissing = snapshot.folders_missing || [];
  state.tracks = snapshot.tracks || [];
  trackIndex = new Map(state.tracks.map((t) => [t.id, t]));
  // Un fichier a pu changer : sa copie audio sera redemandée (le cœur garde le cache disque).
  audioPaths.clear();
  // Sans ça, le prochain réglage enregistré renverrait l'ancienne liste de dossiers.
  EchoLogic.syncFolders(state.settings, snapshot);
  libraryVersion += 1;
  renderAll();
  renderPlaylists();
  scheduleFrames();
  if (state.settings && state.settings.show_covers) prefetchCovers();
}

/// Un seul morceau a changé (modification) : inutile de recharger toute la bibliothèque.
function replaceTrack(track) {
  if (!track) return;
  const i = state.tracks.findIndex((t) => t.id === track.id);
  if (i < 0) return;
  const before = state.tracks[i];
  snapshotSig = "";
  // Seule la pochette change (image tirée d'un clip) : elle est posée sur place. Le même
  // objet est gardé, pour que les listes déjà calculées voient la nouvelle pochette.
  const sameText =
    before.title === track.title && before.artist === track.artist && before.album === track.album &&
    before.duration_ms === track.duration_ms && before.has_audio === track.has_audio;
  if (sameText) {
    Object.assign(before, track);
    if (!coversSoon) coversSoon = setTimeout(() => ((coversSoon = null), repaintCovers()), 300);
    return;
  }
  state.tracks[i] = track;
  trackIndex.set(track.id, track);
  libraryVersion += 1;
  // Plusieurs morceaux changent d'affilée : un seul rendu pour la série.
  if (renderSoon) return;
  renderSoon = setTimeout(() => {
    renderSoon = null;
    renderAll();
    renderPlaylists();
  }, 120);
}
let coversSoon = null;
let renderSoon = null;

async function applyBootstrap(boot) {
  // Réglages importés ou remis à zéro : tout est redessiné, même bibliothèque ou non.
  snapshotSig = "";
  state.settings = boot.settings;
  state.formats = boot.formats || [];
  state.cacheBytes = boot.cache_bytes || 0;
  state.discordReady = !!boot.discord_ready;
  state.version = boot.version || "";
  // Aléatoire et répétition par défaut s'appliquent à chaque démarrage, reprise ou non.
  state.shuffle = !!boot.settings.shuffle_default;
  state.repeat = boot.settings.repeat_default || "off";
  applySettings();
  applySnapshot(boot.snapshot);
  // Le morceau (repris, ou celui que le cœur joue déjà) est en place avant que la
  // fenêtre n'apparaisse : pas de « Rien en cours » suivi d'un changement animé.
  if (boot.native_audio) {
    const adopted = await enableNative();
    if (!adopted) prepareResume();
  } else {
    prepareResume();
  }
  refreshSettingsPanel();
}

/// Prépare la reprise : le morceau et la position de la dernière fois.
function prepareResume() {
  const s = state.settings;
  if (!s.resume_last || !s.last_track) return;
  const track = trackById(s.last_track);
  if (!track) return;
  buildQueueFromLibrary(track.id);
  state.resuming = { id: track.id, position: s.remember_position ? s.last_position_ms : 0 };
  playCurrent({ autoplay: s.last_playing });
}

/* ============================================================
   Branchements
   ============================================================ */

/// Vrai quand le focus a été déplacé au clavier (Tab), faux après un clic : Espace
/// active alors le bouton choisi, sinon il reste « lecture / pause ».
let keyboardFocus = false;

function bindOnce() {
  document.addEventListener("keydown", (e) => e.key === "Tab" && (keyboardFocus = true), true);
  document.addEventListener("pointerdown", () => (keyboardFocus = false), true);
  bindEditor();
  bindPlaylists();
  // Un clic ailleurs, un défilement ou une autre fenêtre ferment le menu d'un morceau.
  document.addEventListener("click", (e) => {
    if (!e.target.closest("#ctx-menu")) closeTrackMenu();
  });
  // Défiler ailleurs ferme le menu ; défiler sa propre liste (playlists), non.
  document.addEventListener(
    "scroll",
    (e) => {
      if (!(e.target instanceof Element) || !e.target.closest("#ctx-menu")) closeTrackMenu();
    },
    true
  );
  window.addEventListener("blur", closeTrackMenu);
  window.addEventListener("resize", closeTrackMenu);
  document.querySelectorAll(".nav-item").forEach((btn) => {
    btn.addEventListener("click", () => switchView(btn.dataset.view));
  });

  el("search").addEventListener("input", (e) => {
    state.search = e.target.value;
    renderLibrary();
  });
  el("sort").addEventListener("change", (e) => {
    state.sort = e.target.value;
    renderLibrary();
  });
  el("min-search").addEventListener("input", (e) => {
    state.minSearch = e.target.value;
    renderMinimal();
  });

  el("btn-add-folder").addEventListener("click", addFolder);
  el("btn-empty-folder").addEventListener("click", addFolder);
  el("btn-banner-close").addEventListener("click", () => {
    el("banner").hidden = true;
    bannerDismissed = state.foldersMissing.join("\n");
  });
  el("btn-rescan").addEventListener("click", rescan);

  el("btn-play").addEventListener("click", togglePlay);
  el("btn-play-big").addEventListener("click", togglePlay);
  el("btn-next").addEventListener("click", () => step(1));
  el("btn-prev").addEventListener("click", () => step(-1));
  el("comp-play").addEventListener("click", togglePlay);
  el("comp-next").addEventListener("click", () => step(1));
  el("comp-prev").addEventListener("click", () => step(-1));
  el("min-play").addEventListener("click", togglePlay);
  el("min-prev").addEventListener("click", () => step(-1));
  el("min-next").addEventListener("click", () => step(1));

  const toggleShuffle = () => {
    state.shuffle = !state.shuffle;
    refreshToggles();
    syncNativeQueue();
  };
  el("btn-shuffle").addEventListener("click", toggleShuffle);
  el("btn-shuffle-listen").addEventListener("click", toggleShuffle);
  el("comp-shuffle").addEventListener("click", toggleShuffle);

  const cycleRepeat = () => {
    const order = ["off", "all", "one"];
    state.repeat = order[(order.indexOf(state.repeat) + 1) % 3];
    refreshToggles();
    syncNativeQueue();
  };
  el("btn-repeat").addEventListener("click", cycleRepeat);

  const toggleMute = () => {
    Sound.setMuted(!Sound.muted);
    refreshToggles();
  };
  el("btn-mute").addEventListener("click", toggleMute);
  el("dock-volume").addEventListener("click", toggleMute);
  el("min-volume").addEventListener("click", toggleMute);

  el("volume").addEventListener("input", (e) => {
    const v = Number(e.target.value);
    e.target.style.setProperty("--p", `${v}%`);
    Sound.setVolume(v / 100);
    saveSettings({ volume: v }, { refresh: false });
  });

  el("seek").addEventListener("input", (e) => {
    state.seeking = true;
    const dur = mediaEl().duration || 0;
    el("time-now").textContent = formatTime((Number(e.target.value) / 1000) * dur * 1000);
  });
  el("seek").addEventListener("change", (e) => {
    seekToRatio(Number(e.target.value) / 1000);
    state.seeking = false;
  });

  bindBar(el("comp-bar"));
  bindBar(el("min-bar"));

  // Une rafale de molette ne change de morceau qu'une fois toutes les 350 ms.
  let lastWheelStep = 0;
  el("carousel").addEventListener("wheel", (e) => {
    if (Math.abs(e.deltaY) < 12) return;
    const now = Date.now();
    if (now - lastWheelStep < 350) return;
    lastWheelStep = now;
    step(e.deltaY > 0 ? 1 : -1);
  }, { passive: true });

  // dock du mode compacte
  el("dock-listen").addEventListener("click", closeSheet);
  el("dock-search").addEventListener("click", () => {
    openSheet(el("library-panel"), "Bibliothèque");
    el("search").focus();
  });
  el("dock-library").addEventListener("click", () => openSheet(el("library-panel"), "Bibliothèque"));
  const openPlaylists = () => {
    openSheet(el("playlists-panel"), "Playlists");
    renderPlaylists();
  };
  el("dock-playlists").addEventListener("click", openPlaylists);
  el("min-playlists").addEventListener("click", openPlaylists);
  el("dock-settings").addEventListener("click", () => openSheet(el("settings-panel"), "Réglages", { flush: true }));
  el("comp-settings").addEventListener("click", () => openSheet(el("settings-panel"), "Réglages", { flush: true }));
  el("comp-queue").addEventListener("click", () => {
    renderQueue();
    const list = el("queue");
    openSheet(list, "File d'attente");
  });

  el("min-settings").addEventListener("click", () => openSheet(el("settings-panel"), "Réglages", { flush: true }));
  el("min-queue").addEventListener("click", () => {
    renderQueue();
    openSheet(el("queue"), "File d'attente");
  });

  const toggleTheme = () => {
    const next = resolvedTheme() === "dark" ? "light" : "dark";
    saveSettings({ theme: next });
  };
  el("comp-theme").addEventListener("click", toggleTheme);
  el("min-theme").addEventListener("click", toggleTheme);

  el("sheet-close").addEventListener("click", closeSheet);
  el("sheet").addEventListener("click", (e) => {
    if (e.target === el("sheet")) closeSheet();
  });

  // Nombre de fichiers d'affilée qui n'ont pas pu être lus : au-delà, on s'arrête.
  let errorStreak = 0;

  Sound.players.forEach((media) => {
    // Seul le lecteur actif compte : celui qui s'éteint dans un fondu est ignoré.
    const on = (name, handler) => media.addEventListener(name, (e) => media === audio && handler(e));
    // WebKit envoie timeupdate quatre fois par seconde : l'affichage ne change qu'à
    // chaque seconde, inutile de redessiner entre-temps.
    let shownSecond = -1;
    on("timeupdate", () => {
      maybeCrossfade();
      const second = Math.floor(media.currentTime);
      if (second === shownSecond) return;
      shownSecond = second;
      updateProgress();
    });
    on("loadedmetadata", () => {
      updateProgress();
      learnDuration(media);
    });
    on("playing", () => {
      errorStreak = 0;
    });
    on("play", () => {
      updateProgress();
      pushPlayback(true);
      renderCarousel();
      renderMinimal();
      renderLibrary();
    });
    on("pause", () => {
      updateProgress();
      pushPlayback(true);
      saveProgress(true);
      renderCarousel();
      renderMinimal();
      renderLibrary();
    });
    on("ended", () => step(1, true));
    on("error", () => {
      if (!currentTrack()) return;
      errorStreak += 1;
      if (errorStreak >= 3) {
        errorStreak = 0;
        media.pause();
        toast("Plusieurs fichiers de suite ne se lisent pas : lecture arrêtée.");
        return;
      }
      toast("Ce fichier n'a pas pu être lu.");
      if (state.settings.next_on_error) setTimeout(() => step(1, true), 600);
    });
  });

  document.addEventListener("keydown", (e) => {
    if (!el("ctx-menu").hidden) {
      if (e.code === "Escape") {
        e.preventDefault();
        closeTrackMenu();
      }
      return;
    }
    // Fenêtre « Modifier » ouverte : le clavier sert à ses champs et boutons, pas au lecteur.
    if (!el("edit-dialog").hidden && e.code !== "Escape") return;
    if (e.code === "Escape" && !el("edit-dialog").hidden) {
      closeEditor();
      return;
    }
    const inField = e.target.tagName === "INPUT" || e.target.tagName === "SELECT" || e.target.tagName === "TEXTAREA";
    if (inField) {
      if (e.code !== "Escape") return;
      // Échap dans un champ : annule la boîte de dialogue, sinon vide la recherche, sinon
      // quitte le champ (et la feuille ou les paroles se ferment au prochain Échap).
      if (!el("modal").hidden && modalCancel) {
        e.preventDefault();
        modalCancel();
      } else if (e.target.type === "search" && e.target.value) {
        e.target.value = "";
        e.target.dispatchEvent(new Event("input", { bubbles: true }));
      } else {
        e.target.blur();
        if (!el("sheet").hidden) closeSheet();
      }
      return;
    }
    // Tant qu'une boîte de dialogue est ouverte, seule Échap agit : pas de lecture cachée.
    if (!el("modal").hidden && e.code !== "Escape") return;
    // Espace sur un bouton atteint au clavier (Tab) : il l'active, comme partout. Après un
    // clic de souris, Espace reste « lecture / pause ».
    if (e.code === "Space" && e.target !== document.body && keyboardFocus &&
        e.target.closest('button, a, [role="button"], [role="switch"], [role="slider"], [tabindex]')) return;
    const media = mediaEl();
    if (e.code === "Space") {
      e.preventDefault();
      togglePlay();
    } else if (e.code === "ArrowRight" && e.shiftKey) {
      step(1);
    } else if (e.code === "ArrowLeft" && e.shiftKey) {
      step(-1);
    } else if (e.code === "ArrowRight") {
      // Durée encore inconnue : on avance quand même (au lieu de revenir au début).
      const end = isFinite(media.duration) && media.duration > 0 ? media.duration : Infinity;
      media.currentTime = Math.min(end, media.currentTime + state.settings.skip_seconds);
    } else if (e.code === "ArrowLeft") {
      media.currentTime = Math.max(0, media.currentTime - state.settings.skip_seconds);
    } else if (e.code === "KeyL" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      toggleLyrics();
    } else if (e.code === "Escape") {
      if (!el("modal").hidden && modalCancel) modalCancel();
      else if (lyricsOpen) toggleLyrics(false);
      else if (!el("sheet").hidden) closeSheet();
    }
  });

  window.addEventListener("beforeunload", () => saveProgress(true));

  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (state.settings.theme === "system") applySettings();
  });
}

function refreshToggles() {
  const shuffle = !!state.shuffle;
  const repeat = state.repeat || "off";
  for (const id of ["btn-shuffle", "btn-shuffle-listen", "comp-shuffle"]) {
    el(id).classList.toggle("is-on", shuffle);
    el(id).setAttribute("aria-pressed", String(shuffle));
  }
  el("btn-repeat").classList.toggle("is-on", repeat !== "off");
  el("btn-repeat").classList.toggle("is-one", repeat === "one");
  el("btn-repeat").setAttribute("aria-pressed", String(repeat !== "off"));
  el("btn-mute").classList.toggle("is-muted", Sound.muted);
  el("btn-mute").setAttribute("aria-pressed", String(Sound.muted));
  el("dock-volume").classList.toggle("on", Sound.muted);
  el("min-volume").classList.toggle("on", Sound.muted);
  // Coupé ou non : l'icône elle-même change, pas seulement sa couleur.
  const volIcon = Sound.muted ? "volume_mute" : "volume_high";
  for (const [id, size] of [["dock-volume", 19], ["min-volume", 18]]) {
    const node = el(id);
    if (node && node.dataset.icon !== volIcon) {
      node.dataset.icon = volIcon;
      node.innerHTML = icon(volIcon, size);
    }
  }
  const label = Sound.muted ? "Remettre le son" : "Couper le son";
  ["dock-volume", "min-volume", "btn-mute"].forEach((id) => el(id) && (el(id).title = label, el(id).setAttribute("aria-label", label)));
  // Trois états pour Répéter : l'infobulle dit lequel.
  const rep = { off: "Répéter : non", all: "Répéter : toute la file", one: "Répéter : ce morceau" }[repeat];
  el("btn-repeat").title = rep;
  el("btn-repeat").setAttribute("aria-label", rep);
}

/// Bibliothèque arrivée avant la fin du démarrage : appliquée juste après.
let pendingLibrary = null;

async function bindBackendEvents() {
  await listen("library-updated", (event) => {
    if (!state.settings) pendingLibrary = event.payload;
    else applySnapshot(event.payload);
  });
  // Analyse des dossiers : une pastille visible dans toutes les dispositions, seulement
  // si l'analyse dure (une analyse éclair ne doit rien faire clignoter).
  let scanTimer = null;
  await listen("scanning", (event) => {
    const pill = el("scan-pill");
    clearTimeout(scanTimer);
    if (event.payload) {
      scanTimer = setTimeout(() => pill && (pill.hidden = false), 600);
    } else {
      if (pill) pill.hidden = true;
      setText(el("track-count"), countLabel(audioTracks().length));
    }
  });
  // Vignettes fabriquées en tâche de fond : posées sur place, par lots.
  await listen("thumbs-ready", async (event) => {
    const entries = Object.entries(event.payload || {});
    // Décodées avant d'être posées : la vignette remplace l'image sans passer par un fond vide.
    await Promise.all(
      entries.map(([, paths]) => {
        if (!paths || !paths.thumb) return null;
        const img = new Image();
        img.src = convertFileSrc(paths.thumb);
        return img.decode().catch(() => {});
      })
    );
    for (const [id, paths] of entries) rememberCover(id, paths);
    repaintCovers();
  });
  // Le cœur a trouvé (ou installé) une mise à jour en tâche de fond.
  await listen("update-status", (event) => showUpdate(event.payload));
  // Plusieurs demandes de fermeture d'affilée (raccourci pressé deux fois) : une seule question.
  let quitAsked = false;
  await listen("close-requested", async () => {
    if (quitAsked) return;
    quitAsked = true;
    try {
      const quit = await ask("Quitter Écho ?", "La musique s'arrêtera.", [
        { label: "Annuler", value: false },
        { label: "Quitter", value: true, primary: true },
      ]);
      if (quit) {
        saveProgress(true);
        await invoke("quit_app");
      }
    } finally {
      quitAsked = false;
    }
  });
}

/* ============================================================
   Paroles
   ============================================================ */

/// Paroles déjà chargées, par morceau.
const lyricsCache = new Map();
let lyricsOpen = false;
let lyricsFor = null;
let lyricsLines = null;
let lyricsActive = -1;
let lyricsTimer = null;
/// Paroles affichées (avec leurs calages), et le mode « Caler » en cours.
let lyricsData = null;
let lyricsCalibrating = false;
/// Défilement à la main : le suivi automatique s'efface quelques secondes.
let lyricsUserScroll = 0;

const LYRICS_SOURCES = { lrc: "Fichier .lrc", file: "Dans le fichier", lrclib: "LRCLIB", lyricsovh: "lyrics.ovh" };

function toggleLyrics(force) {
  const open = force === undefined ? !lyricsOpen : !!force;
  if (open === lyricsOpen) return;
  lyricsOpen = open;
  el("lyrics").hidden = !open;
  ["btn-lyrics", "comp-lyrics", "min-lyrics"].forEach((id) => el(id) && el(id).classList.toggle("on", open));
  if (open) {
    loadLyrics();
    clearInterval(lyricsTimer);
    lyricsTimer = setInterval(syncLyrics, 100);
  } else {
    clearInterval(lyricsTimer);
    lyricsTimer = null;
  }
}

function lyricsMessage(title, text, extra = "") {
  return `<div class="lyr-empty"><div class="lyr-empty-ic">${icon("mic", 26)}</div><div class="lyr-empty-t">${escapeHtml(title)}</div>${
    text ? `<div class="lyr-empty-d">${escapeHtml(text)}</div>` : ""
  }${extra}</div>`;
}

function paintLyricsHead(track) {
  el("lyr-title").textContent = track ? track.title : "Rien en cours";
  el("lyr-artist").textContent = track ? track.artist || "—" : "";
  const cover = el("lyr-cover");
  const bg = el("lyr-bg");
  if (track) {
    fillCover(cover, track, false);
    // Sous un flou de 60 px, la vignette suffit (et se décode tout de suite).
    const pick = () => thumbUrls.get(track.cover) || coverUrls.get(track.cover);
    const paint = (url) => {
      const value = url ? `url("${url}") center / cover no-repeat` : cover.style.background;
      if (bg.style.background !== value) bg.style.background = value;
    };
    if (track.cover && coverUrls.has(track.cover)) paint(pick());
    // Réponse tardive : seulement si ce morceau est toujours celui affiché.
    else if (track.cover) coverSrc(track.cover).then(() => lyricsFor === track.id && paint(pick()));
    else paint(null);
  } else {
    bg.style.background = "";
  }
}

async function loadLyrics(online = false) {
  const track = currentTrack();
  paintLyricsHead(track);
  const body = el("lyr-body");
  el("lyr-src").textContent = "";
  lyricsLines = null;
  lyricsData = null;
  lyricsActive = -1;
  lyricsCalibrating = false;
  paintLyricsBar();
  if (!track) {
    lyricsFor = null;
    body.innerHTML = lyricsMessage("Rien en cours", "Lance un morceau pour voir ses paroles.");
    return;
  }
  lyricsFor = track.id;
  let lyrics = online ? null : lyricsCache.get(track.id);
  if (!lyrics) {
    // Le message d'attente n'apparaît que si la recherche dure : un fichier .lrc répond
    // en quelques millisecondes, le message ne ferait que clignoter.
    const slow = setTimeout(() => {
      if (lyricsFor === track.id) body.innerHTML = `<div class="lyr-loading">Recherche des paroles…</div>`;
    }, 250);
    lyrics = await invoke("get_lyrics", { id: track.id, online }).catch(() => null);
    clearTimeout(slow);
    // Introuvable sans avoir cherché en ligne : on ne le retient pas, la recherche peut venir.
    if (lyrics && (lyrics.source !== "none" || online || state.settings.lyrics_online)) lyricsCache.set(track.id, lyrics);
    if (lyricsFor !== track.id || !lyricsOpen) return;
  }
  renderLyrics(lyrics || { source: "none" });
}

function renderLyrics(lyrics) {
  const body = el("lyr-body");
  const src = LYRICS_SOURCES[lyrics.source];
  lyricsData = lyrics;
  if (!lyricsData.anchors) lyricsData.anchors = [];
  paintLyricsBar();
  if (lyrics.synced && lyrics.synced.length) {
    lyricsLines = lyrics.synced;
    el("lyr-src").textContent = src ? `Synchronisées · ${src}` : "Synchronisées";
    el("lyr-src").title = lyrics.matched ? `Trouvées pour : ${lyrics.matched}` : "";
    body.innerHTML = `<div class="lyr-pad"></div>${lyricsLines
      .map((l, i) => `<p class="lyr-line" data-i="${i}">${escapeHtml(l.text) || "♪"}</p>`)
      .join("")}<div class="lyr-pad"></div>`;
    body.scrollTop = 0;
    syncLyrics(true);
    return;
  }
  if (lyrics.plain) {
    el("lyr-src").textContent = src ? `Non synchronisées · ${src}` : "";
    el("lyr-src").title = lyrics.matched ? `Trouvées pour : ${lyrics.matched}` : "";
    body.innerHTML = `<div class="lyr-plain">${lyrics.plain
      .split(/\r?\n/)
      .map((l) => `<p>${escapeHtml(l) || "&nbsp;"}</p>`)
      .join("")}</div>`;
    body.scrollTop = 0;
    return;
  }
  if (lyrics.instrumental) {
    body.innerHTML = lyricsMessage("Morceau instrumental", "Pas de paroles sur celui-là, profite.");
    return;
  }
  const offer = state.settings.lyrics_online
    ? ""
    : `<button class="primary-btn" data-lyr="online">${icon("search", 15)} Chercher en ligne</button>
       <div class="lyr-empty-d small">Sur LRCLIB, une base libre et gratuite : seuls l'artiste, le titre et la durée sont envoyés.</div>`;
  body.innerHTML = lyricsMessage(
    "Paroles introuvables",
    state.settings.lyrics_online
      ? "Ni le fichier ni LRCLIB n'ont ce morceau. Un fichier .lrc du même nom à côté du morceau marche aussi."
      : "Le fichier n'en contient pas.",
    offer
  );
}

/// La ligne chantée s'illumine et vient au centre.
function syncLyrics(jump = false) {
  if (!lyricsOpen || !lyricsLines) return;
  const anchors = lyricsData ? lyricsData.anchors : [];
  const t = EchoLogic.lyricTime(anchors, mediaEl().currentTime * 1000) + 180;
  let lo = 0;
  let hi = lyricsLines.length - 1;
  let idx = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lyricsLines[mid].ms <= t) {
      idx = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  if (idx === lyricsActive && !jump) return;
  const body = el("lyr-body");
  const old = body.querySelector(".lyr-line.now");
  if (old) old.classList.remove("now");
  body.querySelectorAll(".lyr-line").forEach((p, i) => p.classList.toggle("past", i < idx));
  lyricsActive = idx;
  const line = idx >= 0 ? body.querySelector(`.lyr-line[data-i="${idx}"]`) : null;
  if (line) line.classList.add("now");
  if (Date.now() - lyricsUserScroll < 3500) return;
  const target = line ? line.offsetTop - body.clientHeight * 0.4 + line.offsetHeight / 2 : 0;
  body.scrollTo({ top: Math.max(0, target), behavior: jump || !state.settings.animations ? "auto" : "smooth" });
}

/// Calages du morceau affiché : appliqués tout de suite, gardés pour la prochaine fois.
function setAnchors(anchors) {
  if (!lyricsData || !lyricsFor) return;
  lyricsData.anchors = anchors;
  const cached = lyricsCache.get(lyricsFor);
  if (cached) cached.anchors = anchors;
  invoke("save_lyrics_sync", { id: lyricsFor, anchors }).catch(() => {});
  paintLyricsBar();
  syncLyrics(true);
}

/// Avance ou recule les paroles du passage en cours (un calage est créé s'il n'y en a pas).
function nudgeLyrics(deltaMs) {
  if (!lyricsData) return;
  const now = mediaEl().currentTime * 1000;
  const anchors = [...(lyricsData.anchors || [])];
  const current = anchors.filter((a) => a.at_ms <= now).sort((x, y) => y.at_ms - x.at_ms)[0];
  if (current) {
    const i = anchors.indexOf(current);
    anchors[i] = { at_ms: current.at_ms, lyric_ms: Math.max(0, current.lyric_ms - deltaMs) };
    setAnchors(anchors);
  } else {
    const lyricNow = EchoLogic.lyricTime(anchors, now);
    setAnchors(EchoLogic.addAnchor(anchors, now, Math.max(0, lyricNow - deltaMs)));
  }
}

/// Barre sous l'en-tête : conseil pour un clip, ou commandes du mode « Caler ».
function paintLyricsBar() {
  const bar = el("lyr-bar");
  const tool = el("lyr-sync");
  const track = currentTrack();
  const synced = !!(lyricsData && lyricsData.synced && lyricsData.synced.length);
  tool.hidden = !synced;
  tool.classList.toggle("on", lyricsCalibrating);
  tool.innerHTML = `${icon("clock", 14)}<span>${lyricsCalibrating ? "Terminé" : "Caler"}</span>`;
  const anchors = (lyricsData && lyricsData.anchors) || [];
  if (synced && lyricsCalibrating) {
    bar.hidden = false;
    bar.innerHTML = `<span class="lyr-bar-t">Clique sur la ligne <b>au moment où elle est chantée</b> : tout se cale à partir de là.</span>
      <span class="lyr-bar-ctl">
        <button class="lyr-chip" data-lyr="nudge" data-ms="-500" title="Les paroles arrivent trop tard">−0,5 s</button>
        <button class="lyr-chip" data-lyr="nudge" data-ms="500" title="Les paroles arrivent trop tôt">+0,5 s</button>
        ${anchors.length ? `<button class="lyr-chip" data-lyr="reset">Effacer les calages</button>` : ""}
      </span>`;
  } else if (synced && track && track.video && !anchors.length) {
    bar.hidden = false;
    bar.innerHTML = `<span class="lyr-bar-t">C'est un clip : s'il a une intro ou des scènes en plus, les paroles seront décalées.</span>
      <span class="lyr-bar-ctl"><button class="lyr-chip" data-lyr="calibrate">Caler les paroles</button></span>`;
  } else {
    bar.hidden = true;
    bar.innerHTML = "";
  }
}

function bindLyrics() {
  el("lyr-close").innerHTML = icon("close", 18);
  el("comp-lyrics").innerHTML = icon("mic", 19);
  el("min-lyrics").innerHTML = icon("mic", 18);
  ["btn-lyrics", "comp-lyrics", "min-lyrics"].forEach((id) => el(id).addEventListener("click", () => toggleLyrics()));
  el("lyr-close").addEventListener("click", () => toggleLyrics(false));
  el("lyr-sync").addEventListener("click", () => {
    lyricsCalibrating = !lyricsCalibrating;
    paintLyricsBar();
  });
  el("lyr-bar").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-lyr]");
    if (!btn) return;
    if (btn.dataset.lyr === "calibrate") {
      lyricsCalibrating = true;
      paintLyricsBar();
    } else if (btn.dataset.lyr === "nudge") {
      nudgeLyrics(Number(btn.dataset.ms));
    } else if (btn.dataset.lyr === "reset") {
      setAnchors([]);
    }
  });
  const body = el("lyr-body");
  ["wheel", "touchmove"].forEach((ev) => body.addEventListener(ev, () => (lyricsUserScroll = Date.now()), { passive: true }));
  body.addEventListener("click", async (e) => {
    const line = e.target.closest(".lyr-line");
    if (line && lyricsLines) {
      const l = lyricsLines[Number(line.dataset.i)];
      if (!l) return;
      const now = mediaEl().currentTime * 1000;
      if (lyricsCalibrating) {
        // « Cette ligne est chantée maintenant » : tout se cale à partir d'ici.
        setAnchors(EchoLogic.addAnchor(lyricsData.anchors, now, l.ms));
        toast("Paroles calées. Si le clip coupe le morceau plus loin, recale à la reprise.");
      } else {
        mediaEl().currentTime = EchoLogic.fileTime(lyricsData ? lyricsData.anchors : [], l.ms, now) / 1000;
      }
      lyricsUserScroll = 0;
      syncLyrics(true);
      return;
    }
    if (e.target.closest('[data-lyr="online"]')) {
      saveSettings({ lyrics_online: true });
      loadLyrics(true);
    }
  });
}

/* ============================================================
   Mises à jour
   ============================================================ */

let updateDismissed = "";
let updateBusy = false;
let updateChecking = false;

function showUpdate(status) {
  if (!status) return;
  state.update = status;
  const bar = el("update-bar");
  const v = status.available;
  const key = v ? `${v}:${status.ready ? "ready" : "new"}` : "";
  if (!v || updateDismissed === key) {
    bar.hidden = true;
  } else {
    el("update-text").textContent = status.ready ? `Écho ${v} est installé.` : `Écho ${v} est disponible.`;
    const act = el("update-act");
    act.disabled = updateBusy;
    act.textContent = updateBusy ? "Installation…" : status.ready ? "Redémarrer" : status.can_install ? "Installer" : "Télécharger";
    act.dataset.act = status.ready ? "restart" : status.can_install ? "install" : "page";
    bar.hidden = false;
  }
  if (state.view === "settings" || !el("sheet").hidden) refreshSettingsPanel();
}

async function updateAction(kind) {
  if (kind === "check") {
    updateChecking = true;
    refreshSettingsPanel();
    const status = await invoke("check_update").catch((e) => ({ error: String(e) }));
    updateChecking = false;
    updateDismissed = "";
    showUpdate(status);
    refreshSettingsPanel();
    if (status.error) toast("Vérification impossible pour le moment (connexion ?).");
    else if (!status.available) toast(`Écho ${state.version} est la dernière version.`);
  } else if (kind === "install") {
    updateBusy = true;
    showUpdate(state.update);
    const status = await invoke("install_update").catch((e) => ({ ...state.update, error: String(e) }));
    updateBusy = false;
    showUpdate(status);
    if (status.error) toast(`La mise à jour n'a pas pu s'installer : ${status.error}`);
  } else if (kind === "restart") {
    saveProgress(true);
    await invoke("restart_to_update");
  } else if (kind === "page") {
    invoke("open_releases");
  }
}

function bindUpdates() {
  el("update-close").innerHTML = icon("close", 15);
  el("update-act").addEventListener("click", () => updateAction(el("update-act").dataset.act));
  el("update-close").addEventListener("click", () => {
    const s = state.update;
    if (s && s.available) updateDismissed = `${s.available}:${s.ready ? "ready" : "new"}`;
    el("update-bar").hidden = true;
  });
}

/// Ligne « Mises à jour » des réglages : version, état, et le bon bouton.
function updateRow() {
  const u = state.update;
  let desc;
  let btn;
  if (updateBusy) {
    desc = "Téléchargement et installation…";
    btn = "";
  } else if (u && u.ready) {
    desc = `Écho ${u.available} est installé : il s'applique au redémarrage.`;
    btn = `<button class="primary-btn" data-action="update-restart">Redémarrer</button>`;
  } else if (u && u.available) {
    desc = u.can_install
      ? `Écho ${u.available} est disponible.`
      : `Écho ${u.available} est disponible. Installé par paquet : récupère-le sur la page des versions.`;
    btn = u.can_install
      ? `<button class="primary-btn" data-action="update-install">${icon("download", 15)} Installer</button>`
      : `<button class="ghost-btn" data-action="update-page">${icon("download", 15)} Télécharger</button>`;
  } else {
    desc = updateChecking
      ? "Vérification…"
      : u && u.error
        ? "Dernière vérification impossible (pas de connexion ?)."
        : u
          ? "Tu as la dernière version."
          : "Vérifiées toutes les 6 heures.";
    btn = `<button class="ghost-btn" data-action="update-check"${updateChecking ? " disabled" : ""}>${icon("refresh", 15)} Vérifier maintenant</button>`;
  }
  return rowButton(`Écho ${escapeHtml(state.version || "")}`, desc, btn);
}

/* ============================================================
   Démarrage
   ============================================================ */

async function start() {
  applyIcons(document);
  bindOnce();

  el("comp-settings").innerHTML = icon("settings", 18);
  el("comp-theme").innerHTML = icon("moon", 18);
  el("comp-queue").innerHTML = icon("queue", 19);
  bindLyrics();
  bindUpdates();
  el("comp-prev").innerHTML = icon("prev", 22);
  el("comp-next").innerHTML = icon("next", 22);
  el("comp-play").innerHTML = icon("play", 20);
  el("comp-shuffle").innerHTML = icon("shuffle", 19);
  el("dock-listen").innerHTML = icon("home", 19);
  el("dock-search").innerHTML = icon("search", 19);
  el("dock-library").innerHTML = icon("library", 19);
  el("dock-settings").innerHTML = icon("settings", 19);
  el("dock-playlists").innerHTML = icon("playlist", 19);
  el("min-playlists").innerHTML = icon("playlist", 18);
  el("dock-volume").innerHTML = icon("volume_high", 19);
  el("min-theme").innerHTML = icon("moon", 18);
  el("min-settings").innerHTML = icon("settings", 18);
  el("min-play").innerHTML = icon("play", 18);
  el("min-prev").innerHTML = icon("prev", 18);
  el("min-next").innerHTML = icon("next", 18);
  el("min-queue").innerHTML = icon("queue", 18);
  el("min-volume").innerHTML = icon("volume_high", 18);
  el("sheet-close").innerHTML = icon("close", 18);

  // Les écouteurs d'abord : un scan qui finit pendant le démarrage ne doit pas être perdu.
  await Promise.all([bindMediaKeys(), bindBackendEvents()]).catch(() => {});

  state.playlists = await invoke("get_playlists").catch(() => []);
  const boot = await invoke("bootstrap");
  await applyBootstrap(boot);
  if (pendingLibrary) {
    applySnapshot(pendingLibrary);
    pendingLibrary = null;
  }

  const panel = buildSettingsPanel();
  el("settings-host-normal").appendChild(panel);

  refreshToggles();
  switchView("listen");

}

// La fenêtre apparaît une fois l'interface prête, même si le démarrage a échoué
// (sinon l'utilisateur verrait une fenêtre vide, sans rien pour comprendre).
start()
  .catch((err) => {
    console.error("démarrage d'Écho", err);
    toast("Écho n'a pas pu démarrer correctement.");
  })
  .finally(() => {
    document.body.classList.add("ready");
    // La fenêtre est montrée par le cœur une fois la page dessinée : pas de flash blanc
    // puis sombre, ni d'écran vide le temps du chargement.
    // (Pas de requestAnimationFrame : il peut être suspendu tant que la fenêtre est cachée.)
    setTimeout(() => invoke("ui_ready").catch(() => {}), 30);
  });
