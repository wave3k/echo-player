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
};

const coverCache = new Map();
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

function escape(text) {
  return String(text ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}

function trackById(id) {
  return state.tracks.find((t) => t.id === id) || null;
}
function currentId() {
  return state.queue[state.queueIndex] || null;
}
function currentTrack() {
  return trackById(currentId());
}
function audioTracks() {
  return state.tracks.filter((t) => !t.is_video);
}
function videoTracks() {
  return state.tracks.filter((t) => t.is_video);
}
function mediaEl() {
  const track = currentTrack();
  return track && track.is_video ? video : audio;
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

/// Pose la pochette (ou un dégradé) sur un élément, sans toucher à sa taille.
function paintCover(node, track) {
  if (track && track.cover && coverCache.has(track.cover)) {
    node.style.background = `#222 url("${coverCache.get(track.cover)}") center / cover no-repeat`;
    return;
  }
  const hue = hueFor(track && track.id ? track.id : "x");
  node.style.background = `linear-gradient(140deg, hsl(${hue} 45% 30%), hsl(${(hue + 45) % 360} 45% 17%))`;
}

function noCover(node) {
  node.classList.add("no-cover");
  node.style.background = "";
  node.innerHTML = icon("note", 16);
}

function fillCover(node, track) {
  if (!node) return;
  if (!track || !state.settings.show_covers) {
    noCover(node);
    return;
  }
  node.classList.remove("no-cover");
  node.innerHTML = "";
  paintCover(node, track);

  if (!track.cover || coverCache.has(track.cover)) return;
  // La pochette arrive du disque : on la pose dès qu'elle est prête.
  const wanted = track.cover;
  invoke("get_cover", { id: wanted })
    .then((data) => {
      if (!data) return;
      coverCache.set(wanted, data);
      if (track.cover === wanted) paintCover(node, track);
    })
    .catch(() => {
      /* pochette illisible : le dégradé reste */
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

const FORMAT_NAMES = {
  mp3: "mp3", mp4: "mp4", flac: "flac", m4a: "m4a",
  ogg: "ogg", wav: "wav", webm: "webm", mkv: "mkv",
};

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
  document.documentElement.dataset.text = s.text_size;
  body.style.setProperty("--radius", `${s.radius}px`);
  body.style.setProperty("--radius-soft", `${(s.radius * 0.83).toFixed(1)}px`);
  body.style.setProperty("--radius-md", `${(s.radius * 1.17).toFixed(1)}px`);
  body.style.setProperty("--radius-lg", `${(s.radius * 1.5).toFixed(1)}px`);
  audio.volume = Math.min(1, Math.max(0, s.volume / 100));
  video.volume = audio.volume;
  const volume = el("volume");
  if (volume) volume.value = String(s.volume);
}

let saveTimer = null;
function saveSettings(patch = {}, { refresh = true } = {}) {
  Object.assign(state.settings, patch);
  applySettings();
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
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
    <div class="set-ctl"><span class="switch${value ? " on" : ""}${pending ? " disabled" : ""}" ${attr}><i></i></span></div>
  </div>`;
}

function rowSeg(label, desc, field, options, value) {
  return `<div class="set-row">
    <div class="left"><div class="lab">${label}</div><div class="desc">${desc}</div></div>
    <div class="set-ctl"><span class="seg">${options
      .map((o) => `<button class="${o.value === value ? "on" : ""}" data-set="${field}" data-value="${o.value}">${o.label}</button>`)
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
      <div class="fprev">Écho <span class="fsub">Night Drive — Kavinsky</span></div>
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
        <div class="set-ctl"><span class="slider"><input type="range" min="0" max="20" value="${s.radius}" data-slider="radius" /><span class="hint" id="radius-value">${s.radius} px</span></span></div>
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
      ${rowSwitch("Mémoriser la position", "Même au milieu d'un titre ou d'une vidéo", "remember_position", s.remember_position)}
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
      ${rowSwitch("Fondu enchaîné", "Le titre suivant démarre en douceur", "crossfade", false, null, true)}
      ${rowSwitch("Volume égal", "Rattrape les différences de volume entre les fichiers", "loudness", false, null, true)}
    </div>
  </section>`;
}

function sectionBiblio() {
  const s = state.settings;
  const folders = state.folders.length
    ? state.folders
        .map(
          (f) => `<div class="path-row">
        <span class="path" title="${escape(f)}">${escape(f)}</span>
        <button class="icon-btn" data-remove-folder="${escape(f)}" title="Retirer">${icon("close", 15)}</button>
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
        <div class="set-ctl">${formats}</div>
      </div>
      ${rowSwitch("Actualiser au démarrage", "Vérifie la bibliothèque à chaque ouverture", "refresh_on_start", s.refresh_on_start)}
      ${rowSwitch("Surveiller les changements", "Les nouveaux fichiers apparaissent tout seuls", "watch", false, null, true)}
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
      ${rowSwitch("Icône dans la barre du système", "Pour retrouver la fenêtre à tout moment", "tray", false, null, true)}
      ${rowSwitch("Notification à chaque morceau", "Petit bandeau avec le titre en cours", "notify", false, null, true)}
      ${rowSeg("Quand je ferme la fenêtre", "Ce que fait le bouton de fermeture", "close", [
        { value: "quit", label: "Quitter" },
        { value: "ask", label: "Demander" },
      ], s.ask_before_quit ? "ask" : "quit")}
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
          <div class="about-name">Écho <span class="badge">1.0</span></div>
          <div class="about-desc">Lecteur de musique et de vidéos, pour tes fichiers, sur ton ordinateur.</div>
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

function refreshSettingsPanel() {
  const old = el("settings-panel");
  if (!old) return;
  const fresh = buildSettingsPanel();
  const body = old.querySelector(".settings-body");
  const scroll = body ? body.scrollTop : 0;
  old.replaceWith(fresh);
  const newBody = fresh.querySelector(".settings-body");
  if (newBody) newBody.scrollTop = scroll;
}

const SET_SECTIONS = ["set-apparence", "set-lecture", "set-bibliotheque", "set-systeme", "set-donnees", "set-apropos"];

function bindSettings(panel) {
  panel.querySelector(".settings-nav").addEventListener("click", (e) => {
    const btn = e.target.closest("[data-goto]");
    if (!btn) return;
    const target = el(SET_SECTIONS[Number(btn.dataset.goto)]);
    if (target) target.scrollIntoView({ block: "start", behavior: state.settings.animations ? "smooth" : "auto" });
    panel.querySelectorAll(".settings-nav button").forEach((b) => b.classList.toggle("on", b === btn));
  });

  const body = panel.querySelector(".settings-body");
  body.addEventListener("scroll", () => {
    const top = body.scrollTop + 40;
    let index = 0;
    SET_SECTIONS.forEach((id, i) => {
      const node = el(id);
      if (node && node.offsetTop - 120 <= top) index = i;
    });
    panel.querySelectorAll(".settings-nav button").forEach((b, i) => b.classList.toggle("on", i === index));
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
      saveSettings({ [field]: !state.settings[field] });
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
      } else if (field === "skip_seconds") {
        saveSettings({ skip_seconds: Number(value) });
      } else if (field === "close") {
        saveSettings({ ask_before_quit: value === "ask" });
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
      case "clear-cache":
        state.cacheBytes = await invoke("clear_cover_cache");
        coverCache.clear();
        renderAll();
        refreshSettingsPanel();
        toast("Les pochettes ont été vidées : elles reviendront au prochain affichage.");
        break;
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

function visibleTracks() {
  const q = normalize(state.search);
  let list = audioTracks();
  if (q) {
    list = list.filter(
      (t) => normalize(t.title).includes(q) || normalize(t.artist).includes(q) || normalize(t.album).includes(q)
    );
  }
  const key = state.sort;
  const dir = key === "duration" ? -1 : 1;
  return [...list].sort((a, b) => {
    if (key === "duration") return (a.duration_ms - b.duration_ms) * dir;
    return String(a[key] || "").localeCompare(String(b[key] || ""), "fr", { sensitivity: "base" });
  });
}

function rowHtml(track, index, current, playing) {
  return `
    <div class="row-index">${current && playing ? `<span class="eq">${ICONS.waveform}</span>` : index + 1}</div>
    <div class="row-main">
      <div class="thumb"></div>
      <div class="row-text">
        <div class="row-title">${escape(track.title)}</div>
        <div class="row-artist">${escape(track.artist || "—")}</div>
      </div>
    </div>
    <div class="row-album">${escape(track.album || "—")}</div>
    <div class="row-time">${formatTime(track.duration_ms)}</div>`;
}

function renderLibrary() {
  const listEl = el("list");
  if (!listEl) return;
  const all = visibleTracks();
  state.listOrder = all.map((t) => t.id);

  const spacer = document.createElement("div");
  spacer.className = "list-spacer";
  spacer.style.height = `${all.length * ROW_HEIGHT}px`;
  listEl.replaceChildren(spacer);

  const playing = isPlaying();
  const draw = () => {
    const scrollTop = listEl.scrollTop;
    const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 6);
    const end = Math.min(all.length, Math.ceil((scrollTop + listEl.clientHeight) / ROW_HEIGHT) + 6);
    const frag = document.createDocumentFragment();
    for (let i = start; i < end; i++) {
      const track = all[i];
      const row = document.createElement("div");
      row.className = "row" + (track.id === currentId() ? " is-current" : "") + (track.id === currentId() && playing ? " is-playing" : "");
      row.style.top = `${i * ROW_HEIGHT}px`;
      row.dataset.index = String(i);
      row.innerHTML = rowHtml(track, i, track.id === currentId(), playing);
      fillCover(row.querySelector(".thumb"), track, true);
      row.addEventListener("click", () => playFromList(i));
      frag.appendChild(row);
    }
    spacer.replaceChildren(frag);
  };

  listEl.onscroll = draw;
  draw();
}

function renderVideos() {
  const tiles = el("tiles");
  tiles.replaceChildren();
  const vids = videoTracks();
  for (const track of vids) {
    const tile = document.createElement("button");
    tile.className = "tile";
    tile.innerHTML = `
      <div class="tile-art">${icon("video", 26)}</div>
      <div class="tile-body">
        <div class="tile-title">${escape(track.title)}</div>
        <div class="tile-sub">${escape(track.artist || formatTime(track.duration_ms))}</div>
      </div>`;
    const art = tile.querySelector(".tile-art");
    fillCover(art, track);
    if (!track.cover || !state.settings.show_covers) art.innerHTML = icon("video", 26);
    tile.addEventListener("click", () => openVideo(track));
    tiles.appendChild(tile);
  }
  if (vids.length === 0 && state.folders.length) {
    const p = document.createElement("div");
    p.className = "hint";
    p.textContent = "Aucun clip dans tes dossiers pour l'instant.";
    tiles.appendChild(p);
  }
}

function renderQueue() {
  const queue = el("queue");
  if (!queue) return;
  queue.replaceChildren();
  const ids = state.queue;
  ids.forEach((id, i) => {
    const track = trackById(id);
    if (!track) return;
    const item = document.createElement("button");
    item.className = "queue-item" + (id === currentId() ? " is-current" : "");
    item.innerHTML = `<span class="queue-num">${i + 1}</span>
      <span class="queue-name">${escape(track.title)}${track.artist ? " — " + escape(track.artist) : ""}</span>
      <span class="queue-time">${formatTime(track.duration_ms)}</span>`;
    item.addEventListener("click", () => {
      state.queueIndex = i;
      playCurrent();
    });
    queue.appendChild(item);
  });
  if (ids.length === 0) {
    const p = document.createElement("div");
    p.className = "hint";
    p.textContent = "La file est vide. Choisis un morceau dans la bibliothèque.";
    queue.appendChild(p);
  }
  const count = el("queue-count");
  if (count) count.textContent = ids.length ? `${ids.length} morceaux` : "";
}

/* ============================================================
   Mode compacte : carrousel
   ============================================================ */

function renderCarousel() {
  const box = el("carousel");
  if (!box) return;
  const playing = isPlaying();
  const now = currentId();
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

function albumList() {
  const map = new Map();
  for (const track of audioTracks()) {
    const name = track.album || "Sans album";
    const key = `${normalize(name)}::${normalize(track.artist || "")}`;
    if (!map.has(key)) map.set(key, { key, name, artist: track.artist || "", tracks: [] });
    map.get(key).tracks.push(track);
  }
  return [...map.values()].sort((a, b) => a.name.localeCompare(b.name, "fr", { sensitivity: "base" }));
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
  if (!right) return;

  if (state.minSearch.trim()) {
    const q = normalize(state.minSearch);
    const results = audioTracks()
      .filter((t) => normalize(t.title).includes(q) || normalize(t.artist).includes(q) || normalize(t.album).includes(q))
      .slice(0, 200);
    right.innerHTML = `
      <div class="album-title">Résultats</div>
      <div class="album-meta">${results.length} morceau${results.length > 1 ? "x" : ""} pour « ${escape(state.minSearch)} »</div>
      <div class="album-tracks">${results
        .map((t, i) => minimalTrackRow(t, i, true))
        .join("")}</div>`;
    right.querySelectorAll("[data-min-id]").forEach((node) => {
      node.addEventListener("click", () => playFromResults(node.dataset.minId, results.map((t) => t.id)));
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
    <div class="album-title">${escape(album.name)}</div>
    <div class="album-meta">${album.artist ? escape(album.artist) + " · " : ""}${album.tracks.length} titre${album.tracks.length > 1 ? "s" : ""} · ${formatTime(total)}</div>
    <div class="album-tracks">${album.tracks.map((t, i) => minimalTrackRow(t, i, false)).join("")}</div>`;

  right.querySelectorAll("[data-min-id]").forEach((node) => {
    node.addEventListener("click", () => {
      state.queue = album.tracks.map((t) => t.id);
      state.queueIndex = album.tracks.findIndex((t) => t.id === node.dataset.minId);
      playCurrent();
    });
  });

  const cover = el("min-cover");
  fillCover(cover, album.tracks[0]);
  cover.dataset.album = album.key;

  const albums = albumList();
  const index = albums.findIndex((a) => a.key === album.key);
  const prev = albums[(index - 1 + albums.length) % albums.length];
  const next = albums[(index + 1) % albums.length];
  setPeek(el("peek-left"), prev, album.key);
  setPeek(el("peek-right"), next, album.key);
}

function minimalTrackRow(track, index, showAlbum) {
  const playing = isPlaying();
  const current = track.id === currentId();
  const left = current && playing ? `<span class="playing">${ICONS.waveform}</span>` : `<span class="trk-n">${index + 1}</span>`;
  return `<button class="trk${current ? " is-current" : ""}" data-min-id="${escape(track.id)}">
    ${left}
    <span class="trk-t">${escape(track.title)}${showAlbum && track.album ? ` <span class="hint">${escape(track.album)}</span>` : ""}</span>
    <span class="trk-d">${formatTime(track.duration_ms)}</span>
    <span class="trk-m">${icon("more", 15)}</span>
  </button>`;
}

function setPeek(node, album, currentKey) {
  if (!node) return;
  if (!album || album.key === currentKey) {
    node.hidden = true;
    return;
  }
  node.hidden = false;
  fillCover(node, album.tracks[0]);
  node.onclick = () => {
    state.minimalAlbum = album.key;
    renderMinimal();
  };
}

/* ============================================================
   Lecture
   ============================================================ */

const audio = el("audio");
const video = el("video");

function isPlaying() {
  const track = currentTrack();
  if (!track) return false;
  const media = track.is_video ? video : audio;
  return !media.paused;
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

function openVideo(track) {
  audio.pause();
  el("video-overlay").hidden = false;
  el("video-title").textContent = `${track.title}${track.artist ? " — " + track.artist : ""}`;
  video.src = convertFileSrc(track.path);
  video.play().catch(() => {});
  state.queue = videoTracks().map((t) => t.id);
  state.queueIndex = state.queue.indexOf(track.id);
  pushNowPlaying(track);
  pushPlayback(true);
  renderAll();
}

function closeVideo() {
  el("video-overlay").hidden = true;
  video.pause();
}

function playCurrent({ autoplay = true } = {}) {
  const track = currentTrack();
  if (!track) return;
  if (track.is_video) {
    if (autoplay) {
      openVideo(track);
      return;
    }
    // Reprise d'une vidéo qui ne jouait pas : on garde le titre en cours, sans ouvrir la vue.
    el("video-overlay").hidden = true;
    video.pause();
    state.queue = videoTracks().map((t) => t.id);
    state.queueIndex = state.queue.indexOf(track.id);
    pushNowPlaying(track);
    pushPlayback(false);
    renderAll();
    return;
  }
  el("video-overlay").hidden = true;
  video.pause();
  audio.src = convertFileSrc(track.path);
  if (autoplay) audio.play().catch(() => {});
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
  if (track.is_video && el("video-overlay").hidden) {
    // Une vidéo se regarde : on ouvre la vue au lieu de la lancer en sourdine.
    openVideo(track);
    return;
  }
  const media = track.is_video ? video : audio;
  if (media.paused) media.play().catch(() => {});
  else media.pause();
}

function nextIndex(auto) {
  const n = state.queue.length;
  if (n === 0) return -1;
  if (state.shuffle && n > 1) {
    let i = state.queueIndex;
    while (i === state.queueIndex) i = Math.floor(Math.random() * n);
    return i;
  }
  if (state.queueIndex + 1 < n) return state.queueIndex + 1;
  if (auto && state.repeat === "one") return state.queueIndex;
  if (auto && state.repeat === "all") return 0;
  if (auto) return -1;
  return 0;
}

function step(delta, auto = false) {
  if (state.queue.length === 0) {
    if (state.listOrder.length) playFromList(0);
    return;
  }
  if (delta > 0) {
    const i = nextIndex(auto);
    if (i < 0) {
      audio.pause();
      video.pause();
      return;
    }
    state.queueIndex = i;
  } else {
    state.queueIndex = state.queueIndex - 1 < 0 ? state.queue.length - 1 : state.queueIndex - 1;
  }
  playCurrent();
}

/* ---------------- avancement ---------------- */

let lastProgressPush = 0;
let lastSavedPosition = 0;

function updateProgress() {
  const track = currentTrack();
  const media = track && track.is_video ? video : audio;
  const dur = media.duration || 0;
  const ratio = !state.seeking && isFinite(dur) && dur > 0 ? media.currentTime / dur : null;
  const now = media.currentTime || 0;

  if (ratio !== null) {
    const seek = el("seek");
    if (seek) seek.value = String(Math.round(ratio * 1000));
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
  document.querySelectorAll(".ctl-play, .play-big").forEach((b) => b.classList.toggle("is-playing", playing));
  const compPlay = el("comp-play");
  if (compPlay) {
    compPlay.innerHTML = icon(playing ? "pause" : "play", 20);
    compPlay.classList.toggle("is-playing", playing);
  }
  const minPlay = el("min-play");
  if (minPlay) minPlay.innerHTML = icon(playing ? "pause" : "play", 18);

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
  node.addEventListener("click", (e) => {
    const rect = node.getBoundingClientRect();
    seekToRatio((e.clientX - rect.left) / rect.width);
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
  const media = track && track.is_video ? video : audio;
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
  const media = track && track.is_video ? video : audio;
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
    switch (event.payload) {
      case "play":
        media.play().catch(() => {});
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

function renderNowPlaying() {
  const track = currentTrack();
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

function renderAll() {
  const audios = audioTracks().length;
  const vids = videoTracks().length;
  const count = el("track-count");
  if (count) {
    count.textContent =
      state.tracks.length === 0
        ? "Aucun morceau"
        : `${audios} morceau${audios > 1 ? "x" : ""} · ${vids} vidéo${vids > 1 ? "s" : ""}`;
  }

  el("empty").hidden = state.folders.length > 0;
  const banner = el("banner");
  if (state.foldersMissing.length) {
    banner.hidden = false;
    el("banner-text").textContent =
      state.foldersMissing.length === 1
        ? `Dossier introuvable : ${state.foldersMissing[0]}`
        : `${state.foldersMissing.length} dossiers sont introuvables.`;
  } else {
    banner.hidden = true;
  }

  renderLibrary();
  renderVideos();
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
  if (view === "library") renderLibrary();
}

/* ---------------- surfaces superposées ---------------- */

const HOMES = new Map();

function openSheet(node, title, { flush = false } = {}) {
  if (node && !HOMES.has(node) && node.parentElement) HOMES.set(node, node.parentElement);
  const body = el("sheet-body");
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
  if (node) {
    const home = HOMES.get(node);
    if (home) home.appendChild(node);
    else node.remove();
  }
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
  node._timer = setTimeout(() => (node.hidden = true), 3200);
}

function ask(title, text, options) {
  return new Promise((resolve) => {
    el("modal-title").textContent = title;
    el("modal-text").textContent = text;
    const actions = el("modal-actions");
    actions.replaceChildren();
    options.forEach((option) => {
      const btn = document.createElement("button");
      btn.className = option.primary ? "primary-btn" : "ghost-btn";
      btn.textContent = option.label;
      btn.addEventListener("click", () => {
        el("modal").hidden = true;
        resolve(option.value);
      });
      actions.appendChild(btn);
    });
    el("modal").hidden = false;
  });
}

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

function applySnapshot(snapshot) {
  if (!snapshot) return;
  state.folders = snapshot.folders || [];
  state.foldersMissing = snapshot.folders_missing || [];
  state.tracks = snapshot.tracks || [];
  renderAll();
}

function applyBootstrap(boot) {
  state.settings = boot.settings;
  state.formats = boot.formats || [];
  state.cacheBytes = boot.cache_bytes || 0;
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
  state.shuffle = s.shuffle_default;
  state.repeat = s.repeat_default || "off";
  playCurrent({ autoplay: s.last_playing });
}

/* ============================================================
   Branchements
   ============================================================ */

function bindOnce() {
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
  el("btn-banner-close").addEventListener("click", () => (el("banner").hidden = true));
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
    audio.muted = !audio.muted;
    video.muted = audio.muted;
    refreshToggles();
  };
  el("btn-mute").addEventListener("click", toggleMute);
  el("dock-volume").addEventListener("click", toggleMute);
  el("min-volume").addEventListener("click", toggleMute);

  el("volume").addEventListener("input", (e) => {
    const v = Number(e.target.value);
    audio.volume = v / 100;
    video.volume = v / 100;
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

  el("carousel").addEventListener("wheel", (e) => {
    if (Math.abs(e.deltaY) < 12) return;
    step(e.deltaY > 0 ? 1 : -1);
  }, { passive: true });

  // dock du mode compacte
  el("dock-listen").addEventListener("click", closeSheet);
  el("dock-search").addEventListener("click", () => {
    openSheet(el("library-panel"), "Bibliothèque");
    el("search").focus();
  });
  el("dock-library").addEventListener("click", () => openSheet(el("library-panel"), "Bibliothèque"));
  el("dock-videos").addEventListener("click", () => openSheet(el("tiles"), "Vidéos", { flush: true }));
  el("dock-settings").addEventListener("click", () => openSheet(el("settings-panel"), "Réglages", { flush: true }));
  el("comp-settings").addEventListener("click", () => openSheet(el("settings-panel"), "Réglages", { flush: true }));
  el("comp-queue").addEventListener("click", () => {
    renderQueue();
    const list = el("queue");
    openSheet(list, "File d'attente");
  });

  el("min-settings").addEventListener("click", () => openSheet(el("settings-panel"), "Réglages", { flush: true }));
  el("min-videos").addEventListener("click", () => openSheet(el("tiles"), "Vidéos", { flush: true }));
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

  el("video-close").addEventListener("click", closeVideo);
  el("video-overlay").addEventListener("click", (e) => {
    if (e.target === el("video-overlay")) closeVideo();
  });

  [audio, video].forEach((media) => {
    media.addEventListener("timeupdate", updateProgress);
    media.addEventListener("loadedmetadata", updateProgress);
    media.addEventListener("play", () => {
      updateProgress();
      pushPlayback(true);
      renderCarousel();
      renderMinimal();
      renderLibrary();
    });
    media.addEventListener("pause", () => {
      updateProgress();
      pushPlayback(true);
      saveProgress(true);
      renderCarousel();
      renderMinimal();
      renderLibrary();
    });
    media.addEventListener("ended", () => step(1, true));
    media.addEventListener("error", () => {
      if (currentTrack()) {
        toast("Ce fichier n'a pas pu être lu.");
        if (state.settings.next_on_error) setTimeout(() => step(1, true), 600);
      }
    });
  });

  document.addEventListener("keydown", (e) => {
    if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT") return;
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
      if (!el("sheet").hidden) closeSheet();
    }
  });

  window.addEventListener("beforeunload", () => saveProgress(true));

  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (state.settings.theme === "system") applySettings();
  });
}

function refreshToggles() {
  el("btn-shuffle").classList.toggle("is-on", state.shuffle);
  el("btn-shuffle-listen").classList.toggle("is-on", state.shuffle);
  el("comp-shuffle").classList.toggle("is-on", state.shuffle);
  el("btn-repeat").classList.toggle("is-on", state.repeat !== "off");
  el("btn-repeat").classList.toggle("is-one", state.repeat === "one");
  el("btn-mute").classList.toggle("is-muted", audio.muted);
  el("dock-volume").classList.toggle("on", audio.muted);
  el("min-volume").classList.toggle("on", audio.muted);
}

async function bindBackendEvents() {
  await listen("library-updated", (event) => applySnapshot(event.payload));
  await listen("scanning", (event) => {
    const count = el("track-count");
    if (!count) return;
    if (event.payload) count.textContent = "Analyse des dossiers…";
    else {
      const audios = audioTracks().length;
      const vids = videoTracks().length;
      count.textContent = `${audios} morceau${audios > 1 ? "x" : ""} · ${vids} vidéo${vids > 1 ? "s" : ""}`;
    }
  });
  await listen("close-requested", async () => {
    const quit = await ask("Quitter Écho ?", "La musique s'arrêtera.", [
      { label: "Annuler", value: false },
      { label: "Quitter", value: true, primary: true },
    ]);
    if (quit) {
      saveProgress(true);
      await invoke("quit_app");
    }
  });
}

/* ============================================================
   Démarrage
   ============================================================ */

(async function start() {
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
  el("dock-videos").innerHTML = icon("video", 19);
  el("dock-settings").innerHTML = icon("settings", 19);
  el("dock-volume").innerHTML = icon("volume_high", 19);
  el("min-theme").innerHTML = icon("moon", 18);
  el("min-settings").innerHTML = icon("settings", 18);
  el("min-play").innerHTML = icon("play", 18);
  el("min-queue").innerHTML = icon("queue", 18);
  el("min-volume").innerHTML = icon("volume_high", 18);
  el("sheet-close").innerHTML = icon("close", 18);
  el("video-close").innerHTML = icon("close", 18);

  const boot = await invoke("bootstrap");
  applyBootstrap(boot);

  const panel = buildSettingsPanel();
  el("settings-host-normal").appendChild(panel);

  refreshToggles();
  switchView("listen");

  bindMediaKeys().catch(() => {});
  bindBackendEvents().catch(() => {});

  // La fenêtre apparaît quand tout est en place.
  document.body.classList.add("ready");
})();
