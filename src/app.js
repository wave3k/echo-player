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
const coverPending = new Map();

function coverSrc(id) {
  if (coverUrls.has(id)) return Promise.resolve(coverUrls.get(id));
  if (!coverPending.has(id)) {
    const request = invoke("get_cover_path", { id })
      .then((path) => {
        coverUrls.set(id, path ? convertFileSrc(path) : null);
        return coverUrls.get(id);
      })
      .catch(() => null)
      .finally(() => coverPending.delete(id));
    coverPending.set(id, request);
  }
  return coverPending.get(id);
}

/// Pose la pochette (ou un dégradé) sur un élément, sans toucher à sa taille.
/// Sans pochette du tout, le dégradé porte le titre et l'artiste.
function paintCover(node, track, label = true) {
  const url = track && track.cover ? coverUrls.get(track.cover) : null;
  if (url) {
    node.style.background = `#222 url("${url}") center / cover no-repeat`;
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
  const shown = `${track.id}|${track.cover || ""}|${coverUrls.has(track.cover) ? 1 : 0}|${label}`;
  if (node.dataset.shown === shown && !node.classList.contains("no-cover")) return;
  node.dataset.shown = shown;
  node.classList.remove("no-cover");
  node.innerHTML = "";
  // Les petites vignettes des listes restent sans texte : illisible, et redessiné au défilement.
  if (node.classList.contains("thumb")) label = false;
  paintCover(node, track, label);

  if (!track.cover || coverUrls.has(track.cover)) return;
  // La pochette est reconnue : on la pose dès que son adresse est prête.
  const wanted = track.cover;
  coverSrc(wanted).then((url) => {
    if (url && track.cover === wanted) paintCover(node, track, label);
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
    (f) => `<button class="fontcard${s.font === f.id ? " sel" : ""}" data-set="font" data-value="${f.id}">
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
      ${rowButton("Pochettes en cache", `${mo} Mo sur le disque`, `<button class="ghost-btn" data-action="clear-cache">Vider</button>`)}
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
          <div class="about-name">Écho <span class="badge">1.1.1</span></div>
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

function refreshSettingsPanel() {
  const old = el("settings-panel");
  if (!old) return;
  // Le panneau est reconstruit : on garde la position de défilement et le focus clavier.
  const active = document.activeElement;
  const focusKey = active && old.contains(active) ? controlSelector(active) : null;
  const oldNav = document.querySelector(".settings-nav");
  const fresh = buildSettingsPanel();
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
        frameTried.clear();
        state.cacheBytes = cleared.cache_bytes;
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
          applyBootstrap(boot);
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
        applyBootstrap(boot);
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
    const first = Math.max(0, Math.floor(top / rowHeight) - 6);
    const last = Math.min(count, Math.ceil((top + container.clientHeight) / rowHeight) + 6);
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
  const queue = el("queue");
  if (!queue) return;
  const ids = state.queue;
  const count = el("queue-count");
  if (count) count.textContent = ids.length ? `${ids.length} morceaux` : "";

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

function renderMinimal() {
  const right = el("min-right");
  if (!right || state.settings.mode !== "minimal") return;

  if (state.minSearch.trim()) {
    const q = normalize(state.minSearch);
    const matches = audioTracks().filter((t) => searchKey(t).includes(q));
    const results = matches.slice(0, 200);
    const capped = matches.length > results.length ? " · 200 premiers affichés" : "";
    right.innerHTML = `
      <div class="album-title">Résultats</div>
      <div class="album-meta">${matches.length} morceau${matches.length > 1 ? "x" : ""} pour « ${escapeHtml(state.minSearch)} »${capped}</div>
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
    right.innerHTML = `<div class="album-title">Aucun album</div>
      <div class="album-meta">Choisis un dossier dans les réglages, ou ajoute un morceau à ta bibliothèque.</div>`;
    return;
  }

  const total = album.tracks.reduce((sum, t) => sum + (t.duration_ms || 0), 0);
  right.innerHTML = `
    <div class="album-title">${escapeHtml(album.name)}</div>
    <div class="album-meta">${album.artist ? escapeHtml(album.artist) + " · " : ""}${album.tracks.length} titre${album.tracks.length > 1 ? "s" : ""} · ${formatTime(total)}</div>
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

/// Sous Linux, le lecteur audio de WebKitGTK n'accepte pas asset:// : on lui donne
/// l'adresse file:// du morceau. Ailleurs (WebView2), le protocole d'asset fonctionne.
const LINUX_WEBKIT = /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent) && !window.chrome;

// Sous Windows, les fichiers viennent d'une autre origine (asset) : le traitement du son
// et la capture d'images de vidéo demandent le CORS, que le protocole d'asset accorde.
if (!LINUX_WEBKIT) Sound.players.forEach((p) => (p.crossOrigin = "anonymous"));

/// Durée inconnue des tags (fichier sans en-tête complet) : on garde celle du lecteur.
function learnDuration(media) {
  const track = currentTrack();
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
  if (!state.settings.notify || document.hasFocus()) return;
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
  state.queueIndex = ids.indexOf(id);
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
  audio = Sound.element;
  audio.src = mediaSrc(track.path);
  Sound.beginTrack(track.id);
  if (autoplay) {
    startPlayback();
    notifyTrack(track);
  }
  pushNowPlaying(track);
  pushPlayback(true);
  renderAll();
  if (state.resuming && state.resuming.id === track.id && state.settings.remember_position) {
    const target = state.resuming.position;
    state.resuming = null;
    const apply = () => {
      if (isFinite(audio.duration) && audio.duration > 0) {
        audio.currentTime = Math.min(target / 1000, Math.max(0, audio.duration - 1));
      }
      audio.removeEventListener("loadedmetadata", apply);
    };
    audio.addEventListener("loadedmetadata", apply);
  }
}

function togglePlay() {
  const track = currentTrack();
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
  if (!seconds || Sound.fading || audio.paused || state.repeat === "one") return;
  const duration = audio.duration;
  if (!isFinite(duration) || duration < seconds * 3 || duration - audio.currentTime > seconds) return;
  const i = EchoLogic.nextIndex(playContext(), true);
  if (i < 0 || i === state.queueIndex) return;
  const next = trackById(state.queue[i]);
  if (!next || next.has_audio === false) return;
  if (state.shuffle) shuffleBag.history.push(state.queueIndex);
  state.queueIndex = i;
  Sound.other.src = mediaSrc(next.path);
  audio = Sound.crossfade(seconds);
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
    const compBar = el("comp-bar");
    if (compBar) compBar.firstElementChild.style.width = `${ratio * 100}%`;
    const minBar = el("min-bar");
    if (minBar) minBar.firstElementChild.style.width = `${ratio * 100}%`;
  }

  const nowText = formatTime(now * 1000);
  const totalText = formatTime((dur || 0) * 1000);
  ["time-now", "comp-now", "min-now-time"].forEach((id) => {
    const node = el(id);
    if (node) node.textContent = nowText;
  });
  ["time-total", "comp-total", "min-total-time"].forEach((id) => {
    const node = el(id);
    if (node) node.textContent = totalText;
  });

  const playing = !media.paused;
  // Les boutons ne changent qu'au passage lecture ↔ pause, pas à chaque seconde.
  if (playing !== lastPlayingShown) {
    lastPlayingShown = playing;
    document.querySelectorAll(".ctl-play, .play-big").forEach((b) => b.classList.toggle("is-playing", playing));
    el("btn-play").setAttribute("aria-label", playing ? "Pause" : "Lecture");
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
  if (!node) return;
  node.classList.remove(cls);
  void node.offsetWidth;
  node.classList.add(cls);
}

function renderNowPlaying() {
  const track = currentTrack();
  if ((track ? track.id : null) !== lastNowId) {
    lastNowId = track ? track.id : null;
    ["listen-cover", "mini-cover", "min-mini-cover"].forEach((id) => playAnim(el(id), "swap"));
    ["listen-title", "listen-artist", "comp-title", "comp-artist", "mini-title", "mini-artist", "min-title", "min-artist"]
      .forEach((id) => playAnim(el(id), "swap-text"));
  }
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
      node.style.background = "";
      node.innerHTML = "";
    });
  }
}

let bannerDismissed = null;

function renderAll() {
  const audios = audioTracks().length;
  const count = el("track-count");
  if (count) {
    count.textContent =
      state.tracks.length === 0
        ? "Aucun morceau"
        : `${audios} morceau${audios > 1 ? "x" : ""}`;
  }

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

function toast(text) {
  let node = el("toast");
  if (!node) {
    node = document.createElement("div");
    node.id = "toast";
    node.className = "banner";
    node.style.bottom = "150px";
    document.querySelector(".app").appendChild(node);
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
    node.style.background = `#222 url("${convertFileSrc(pickedPath)}") center / cover no-repeat`;
    return;
  }
  if (!coverName) {
    node.style.background = DEFAULT_COVER;
    return;
  }
  coverSrc(coverName).then((url) => {
    node.style.background = url ? `#222 url("${url}") center / cover no-repeat` : DEFAULT_COVER;
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
  toast(next ? "Joué juste après." : "Ajouté à la file.");
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
        <div class="album-meta">${tracks.length} titre${tracks.length > 1 ? "s" : ""}${total ? " · " + formatTime(total) : ""}</div>
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
      for (const [id, path] of Object.entries(paths || {})) {
        if (coverUrls.get(id)) continue;
        const url = convertFileSrc(path);
        coverUrls.set(id, url);
        fresh.push(url);
      }
      // Décodées d'avance (les premières seulement) : pas de case vide à l'affichage.
      fresh.slice(0, 400).forEach((url) => {
        const img = new Image();
        img.decoding = "async";
        img.src = url;
      });
      if (fresh.length) {
        libShown = null;
        renderAll();
      }
    })
    .catch(() => {})
    .finally(() => {
      coverPrefetch = null;
    });
  return coverPrefetch;
}

function applySnapshot(snapshot) {
  if (!snapshot) return;
  state.folders = snapshot.folders || [];
  state.foldersMissing = snapshot.folders_missing || [];
  state.tracks = snapshot.tracks || [];
  trackIndex = new Map(state.tracks.map((t) => [t.id, t]));
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
  state.tracks[i] = track;
  trackIndex.set(track.id, track);
  libraryVersion += 1;
  // Plusieurs pochettes arrivent d'affilée : un seul rendu pour la série.
  if (renderSoon) return;
  renderSoon = setTimeout(() => {
    renderSoon = null;
    renderAll();
    renderPlaylists();
  }, 120);
}
let renderSoon = null;

function applyBootstrap(boot) {
  state.settings = boot.settings;
  state.formats = boot.formats || [];
  state.cacheBytes = boot.cache_bytes || 0;
  state.discordReady = !!boot.discord_ready;
  // Aléatoire et répétition par défaut s'appliquent à chaque démarrage, reprise ou non.
  state.shuffle = !!boot.settings.shuffle_default;
  state.repeat = boot.settings.repeat_default || "off";
  applySettings();
  applySnapshot(boot.snapshot);
  prepareResume();
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

function bindOnce() {
  bindEditor();
  bindPlaylists();
  // Un clic ailleurs, un défilement ou une autre fenêtre ferment le menu d'un morceau.
  document.addEventListener("click", (e) => {
    if (!e.target.closest("#ctx-menu")) closeTrackMenu();
  });
  document.addEventListener("scroll", closeTrackMenu, true);
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

  const toggleShuffle = () => {
    state.shuffle = !state.shuffle;
    refreshToggles();
  };
  el("btn-shuffle").addEventListener("click", toggleShuffle);
  el("btn-shuffle-listen").addEventListener("click", toggleShuffle);
  el("comp-shuffle").addEventListener("click", toggleShuffle);

  const cycleRepeat = () => {
    const order = ["off", "all", "one"];
    state.repeat = order[(order.indexOf(state.repeat) + 1) % 3];
    refreshToggles();
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
    if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT") return;
    // Tant qu'une boîte de dialogue est ouverte, seule Échap agit : pas de lecture cachée.
    if (!el("modal").hidden && e.code !== "Escape") return;
    const media = mediaEl();
    if (e.code === "Space") {
      e.preventDefault();
      togglePlay();
    } else if (e.code === "ArrowRight" && e.shiftKey) {
      step(1);
    } else if (e.code === "ArrowLeft" && e.shiftKey) {
      step(-1);
    } else if (e.code === "ArrowRight") {
      media.currentTime = Math.min(media.duration || 0, media.currentTime + state.settings.skip_seconds);
    } else if (e.code === "ArrowLeft") {
      media.currentTime = Math.max(0, media.currentTime - state.settings.skip_seconds);
    } else if (e.code === "Escape") {
      if (!el("modal").hidden && modalCancel) modalCancel();
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
  ["dock-volume", "min-volume"].forEach((id) => el(id) && (el(id).title = label, el(id).setAttribute("aria-label", label)));
}

/// Bibliothèque arrivée avant la fin du démarrage : appliquée juste après.
let pendingLibrary = null;

async function bindBackendEvents() {
  await listen("library-updated", (event) => {
    if (!state.settings) pendingLibrary = event.payload;
    else applySnapshot(event.payload);
  });
  await listen("scanning", (event) => {
    const count = el("track-count");
    if (!count) return;
    if (event.payload) count.textContent = "Analyse des dossiers…";
    else {
      const audios = audioTracks().length;
      count.textContent = `${audios} morceau${audios > 1 ? "x" : ""}`;
    }
  });
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
   Démarrage
   ============================================================ */

async function start() {
  applyIcons(document);
  bindOnce();

  el("comp-settings").innerHTML = icon("settings", 18);
  el("comp-theme").innerHTML = icon("moon", 18);
  el("comp-queue").innerHTML = icon("queue", 19);
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
  el("min-queue").innerHTML = icon("queue", 18);
  el("min-volume").innerHTML = icon("volume_high", 18);
  el("sheet-close").innerHTML = icon("close", 18);

  // Les écouteurs d'abord : un scan qui finit pendant le démarrage ne doit pas être perdu.
  await Promise.all([bindMediaKeys(), bindBackendEvents()]).catch(() => {});

  state.playlists = await invoke("get_playlists").catch(() => []);
  const boot = await invoke("bootstrap");
  applyBootstrap(boot);
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
  .finally(() => document.body.classList.add("ready"));
